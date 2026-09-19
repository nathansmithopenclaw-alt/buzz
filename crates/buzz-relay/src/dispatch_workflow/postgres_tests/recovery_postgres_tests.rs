use super::*;

#[tokio::test]
#[ignore = "requires isolated Postgres"]
async fn missing_receipt_skips_only_affected_issue_and_recovers_after_receipt_restore() {
    let f = Fixture::new().await;
    f.record(
        f.workspace,
        DispatchRecordPayload::UnownedAlert {
            workspace: f.workspace,
            channel: None,
            after_hours: 24,
            enabled: true,
        },
    )
    .await;
    let legacy = f.issue("Legacy without receipt", false, 25).await;
    f.issue("Normal reminder", false, 25).await;
    let issue: IssueRoot = serde_json::from_str(&legacy.content).unwrap();
    sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND kind=40027 AND content::jsonb->>'event_id'=$2")
        .bind(f.community.as_uuid()).bind(legacy.id.to_hex()).execute(&f.pool).await.unwrap();
    for _ in 0..2 {
        f.invoke(
            f.workspace,
            json!({"id":"capture","action":"dispatch.escalate"}),
            None,
        )
        .await;
    }
    let history = f.history(f.workspace).await;
    let skips = history
        .iter()
        .filter(|e| e.kind.as_u16() == 40023 && e.content.contains("no admission receipt"))
        .collect::<Vec<_>>();
    assert_eq!(
        skips.len(),
        1,
        "missing receipt must produce one visible skip, not abort the tick"
    );
    assert!(skips[0]
        .content
        .contains(&format!("{}:unowned-24h", issue.issue_id)));
    let messages = f
        .state
        .db
        .query_events(&buzz_db::EventQuery {
            kinds: Some(vec![9]),
            channel_id: Some(f.channel),
            limit: Some(100),
            ..buzz_db::EventQuery::for_community(f.community)
        })
        .await
        .unwrap();
    let digests = messages
        .iter()
        .filter(|e| {
            e.event
                .content
                .starts_with("Work that still needs an owner:")
        })
        .collect::<Vec<_>>();
    assert_eq!(digests.len(), 1);
    assert!(digests[0].event.content.contains("Normal reminder"));
    assert!(!digests[0].event.content.contains("Legacy without receipt"));
    // Restore the fixture's authentic admission evidence. The diagnostic skip
    // must not consume the successful issue/threshold alert claim.
    sqlx::query("UPDATE events SET deleted_at=NULL WHERE community_id=$1 AND kind=40027 AND content::jsonb->>'event_id'=$2")
        .bind(f.community.as_uuid()).bind(legacy.id.to_hex()).execute(&f.pool).await.unwrap();
    f.invoke(
        f.workspace,
        json!({"id":"capture","action":"dispatch.escalate"}),
        None,
    )
    .await;
    assert!(f
        .history(f.workspace)
        .await
        .iter()
        .any(
            |e| parse_dispatch_action(e, Timestamp::now().as_secs()).is_ok_and(|a| a
                .action
                .issue_id
                == issue.issue_id
                && a.action.action == DispatchActionKind::Escalate)
        ));
}

#[tokio::test]
#[ignore = "requires isolated Postgres"]
async fn lifecycle_backlog_drains_oldest_admission_first_and_consumed_work_is_gated() {
    let f = Fixture::new().await;
    let mut roots = Vec::new();
    for i in 1..=33u64 {
        let root = f.issue(&format!("Backlog {i}"), false, 50).await;
        let issue: IssueRoot = serde_json::from_str(&root.content).unwrap();
        let mut task = TaskEnvelope {
            v: 3,
            task_id: Uuid::from_u128(u128::from(i)),
            status: TaskStatus::Assigned,
            assignee: f.agent.public_key().to_hex(),
            assigner: f.owner.public_key().to_hex(),
            title: issue.title.clone(),
            deliverables: vec![],
            verification: TaskVerification {
                method: "Independent review".into(),
                evidence_ref: None,
            },
            blocked_on: None,
            notes: None,
            tracker_issue_id: Some(issue.issue_id),
            dispatch_request_id: None,
            authority: Some(TaskAuthority {
                assignment_event_id: None,
                previous_event_id: None,
                reviewer: Some(f.owner.public_key().to_hex()),
            }),
        };
        let assignment = build_task_message(f.channel, "Assignment", &task, None, &[])
            .unwrap()
            .sign_with_keys(&f.owner)
            .unwrap();
        f.save(&assignment, f.channel).await;
        let link = build_tracker_revision(
            f.workspace,
            &IssueRevision {
                v: 1,
                op: "revise".into(),
                issue_id: issue.issue_id,
                root_event: root.id.to_hex(),
                parent_event: root.id.to_hex(),
                revision: 1,
                set: RevisionSet::default(),
                clear: vec![],
                relations: SetOperations::default(),
                external_refs: SetOperations {
                    add: vec![ExternalRef {
                        kind: "task_event".into(),
                        reference: assignment.id.to_hex(),
                    }],
                    remove: vec![],
                },
                reason: None,
                request_id: None,
                import: None,
            },
        )
        .unwrap()
        .sign_with_keys(&f.owner)
        .unwrap();
        f.save(&link, f.workspace).await;
        task.status = TaskStatus::PickedUp;
        let authority = task.authority.as_mut().unwrap();
        authority.assignment_event_id = Some(assignment.id.to_hex());
        authority.previous_event_id = Some(assignment.id.to_hex());
        let pickup = build_task_message(f.channel, "Pickup", &task, None, &[])
            .unwrap()
            .sign_with_keys(&f.agent)
            .unwrap();
        // UUID order opposes receipt order. Author clock stays the same.
        f.save_observed_hours_ago(&pickup, f.channel, i).await;
        roots.push(issue.issue_id);
    }
    let action = json!({"id":"capture","action":"dispatch.escalate"});
    f.invoke(f.workspace, action.clone(), None).await;
    let fold = preliminary_planner(&f.state, f.community, f.workspace)
        .await
        .unwrap()
        .issues(f.workspace);
    assert_eq!(
        fold.issues[&(f.workspace, roots[0])].status,
        IssueStatus::Todo,
        "newest admission must wait past the first 32"
    );
    assert_eq!(
        fold.issues[&(f.workspace, roots[32])].status,
        IssueStatus::InProgress,
        "oldest admission must lead regardless of task UUID"
    );
    assert_eq!(
        fold.issues
            .values()
            .filter(|i| i.status == IssueStatus::InProgress)
            .count(),
        32
    );
    let pickup = f
        .history(f.channel)
        .await
        .into_iter()
        .filter_map(|event| TaskEnvelopeEvent::from_event(&event).ok())
        .find(|task| {
            task.envelope.task_id == Uuid::from_u128(1)
                && task.envelope.status == TaskStatus::PickedUp
        })
        .unwrap();
    let mut done = pickup.envelope;
    done.status = TaskStatus::Done;
    done.authority.as_mut().unwrap().previous_event_id = Some(pickup.event_id);
    let done = build_task_message(f.channel, "Done before recovery", &done, None, &[])
        .unwrap()
        .sign_with_keys(&f.agent)
        .unwrap();
    f.save(&done, f.channel).await;
    f.invoke(f.workspace, action, None).await;
    let channels = discover_scopes(&f.state, f.community, f.workspace)
        .await
        .unwrap();
    f.state
        .db
        .dispatch_workflow_transaction(f.community, &channels, &[], None, |snapshot| {
            let mut p = Planner::new(&f.state, snapshot);
            assert!(
                !p.has_pending_lifecycle(f.workspace, None),
                "consumed lifecycle work must skip the expensive fold"
            );
            p.project_lifecycle(f.workspace, None)?;
            assert!(p.batch.events.is_empty());
            assert_eq!(
                p.issues(f.workspace)
                    .issues
                    .values()
                    .filter(|i| i.status == IssueStatus::InProgress)
                    .count(),
                32
            );
            assert_eq!(
                p.issues(f.workspace).issues[&(f.workspace, roots[0])].status,
                IssueStatus::InReview
            );
            Ok(p.batch)
        })
        .await
        .unwrap();
}
