use super::*;

async fn alerts(f: &Fixture) {
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
}

async fn digests(f: &Fixture) -> Vec<buzz_core::StoredEvent> {
    f.state
        .db
        .query_events(&buzz_db::EventQuery {
            kinds: Some(vec![9]),
            channel_id: Some(f.channel),
            limit: Some(100),
            ..buzz_db::EventQuery::for_community(f.community)
        })
        .await
        .unwrap()
        .into_iter()
        .filter(|e| {
            e.event
                .content
                .starts_with("Work that still needs an owner:")
        })
        .collect()
}

#[tokio::test]
#[ignore = "requires isolated Postgres"]
async fn member_forged_action_cannot_suppress_escalation() {
    let f = Fixture::new().await;
    let root = f.issue("Must alert despite forged action", false, 25).await;
    let issue: IssueRoot = serde_json::from_str(&root.content).unwrap();
    let forged = build_dispatch_action(
        f.workspace,
        &DispatchAction {
            v: 1,
            issue_id: issue.issue_id,
            root_event: root.id.to_hex(),
            request_id: threshold_request_id(issue.issue_id, "unowned-24h", 0),
            action: DispatchActionKind::Escalate,
            threshold_id: Some("unowned-24h".into()),
            observed_tip: Some(root.id.to_hex()),
            result: "Already alerted".into(),
        },
    )
    .unwrap()
    .sign_with_keys(&f.agent)
    .unwrap();
    f.save(&forged, f.workspace).await;
    alerts(&f).await;
    let action = json!({"id":"capture","action":"dispatch.escalate"});
    f.invoke(f.workspace, action.clone(), None).await;
    assert_eq!(
        digests(&f).await.len(),
        1,
        "non-relay member action must not suppress digest"
    );
    f.invoke(f.workspace, action, None).await;
    assert_eq!(
        digests(&f).await.len(),
        1,
        "relay claim still suppresses real duplicate"
    );
}

#[tokio::test]
#[ignore = "requires isolated Postgres"]
async fn member_claimed_capture_is_visible_and_revocation_does_not_suppress() {
    let f = Fixture::new().await;
    let source = EventBuilder::new(Kind::Custom(9), "Source falsely claimed by a member")
        .tags([tag("h", &f.channel.to_string()).unwrap()])
        .sign_with_keys(&f.owner)
        .unwrap();
    f.save(&source, f.channel).await;
    let template = f.issue("Root template", false, 0).await;
    let mut root: IssueRoot = serde_json::from_str(&template.content).unwrap();
    root.issue_id = Uuid::new_v4();
    root.identifier = format!("FORGED-{}", root.issue_id.simple());
    root.source_event_id = Some(source.id.to_hex());
    let forged = build_tracker_issue(f.workspace, &root)
        .unwrap()
        .sign_with_keys(&f.agent)
        .unwrap();
    f.save(&forged, f.workspace).await;
    f.record(
        f.channel,
        DispatchRecordPayload::Capture {
            channel: f.channel,
            workspace: f.workspace,
            project: None,
            enabled: true,
        },
    )
    .await;
    let pin = f.pin(&source, "member-claim").await;
    let action = json!({"id":"capture","action":"dispatch.create_issue"});
    for _ in 0..2 {
        f.invoke(f.channel, action.clone(), Some(&pin)).await;
    }
    let outcomes = f
        .history(f.channel)
        .await
        .into_iter()
        .filter(|e| {
            e.kind.as_u16() == 40023
                && e.pubkey == f.state.relay_keypair.public_key()
                && e.content
                    .contains(&format!("skipped: existing issue {}", root.issue_id))
        })
        .count();
    assert_eq!(
        outcomes, 1,
        "member-claimed root must produce one visible skip naming the issue"
    );
    sqlx::query("UPDATE channel_members SET removed_at=now() WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3")
        .bind(f.community.as_uuid()).bind(f.workspace).bind(f.agent.public_key().to_bytes().as_slice())
        .execute(&f.pool).await.unwrap();
    f.invoke(f.channel, action, Some(&pin)).await;
    assert!(
        f.history(f.workspace).await.iter().any(|e| {
            e.kind.as_u16() == 40020
                && e.pubkey == f.state.relay_keypair.public_key()
                && e.content.contains(&source.id.to_hex())
        }),
        "removed member's root must not suppress authorized capture"
    );
}

fn status_revision(
    f: &Fixture,
    root: &Event,
    parent: &Event,
    revision: u64,
    set: RevisionSet,
    created_at: u64,
) -> Event {
    let issue: IssueRoot = serde_json::from_str(&root.content).unwrap();
    build_tracker_revision(
        f.workspace,
        &IssueRevision {
            v: 1,
            op: "revise".into(),
            issue_id: issue.issue_id,
            root_event: root.id.to_hex(),
            parent_event: parent.id.to_hex(),
            revision,
            set,
            clear: vec![],
            relations: SetOperations::default(),
            external_refs: SetOperations::default(),
            reason: None,
            request_id: None,
            import: None,
        },
    )
    .unwrap()
    .custom_created_at(Timestamp::from(created_at))
    .sign_with_keys(&f.agent)
    .unwrap()
}

#[tokio::test]
#[ignore = "requires isolated Postgres"]
async fn backdated_status_revision_uses_receipt_age_and_title_edits_do_not_reset_it() {
    let f = Fixture::new().await;
    alerts(&f).await;
    let root = f.issue("Backdated state", false, 100).await;
    let now = Timestamp::now().as_secs();
    let recent = status_revision(
        &f,
        &root,
        &root,
        1,
        RevisionSet {
            status: Some(IssueStatus::Triage),
            ..Default::default()
        },
        now - 48 * 3600,
    );
    f.save(&recent, f.workspace).await;
    let action = json!({"id":"capture","action":"dispatch.escalate"});
    f.invoke(f.workspace, action.clone(), None).await;
    assert!(
        digests(&f).await.is_empty(),
        "backdated revision must not manufacture 24 hours of relay-observed age"
    );

    // The relay observed this next state change 25h ago despite a recent author clock.
    let old = status_revision(
        &f,
        &root,
        &recent,
        2,
        RevisionSet {
            status: Some(IssueStatus::Todo),
            ..Default::default()
        },
        now,
    );
    f.save_observed_hours_ago(&old, f.workspace, 25).await;
    let title_only = status_revision(
        &f,
        &root,
        &old,
        3,
        RevisionSet {
            title: Some("Recent title; old observed state".into()),
            ..Default::default()
        },
        now + 1,
    );
    f.save(&title_only, f.workspace).await;
    f.invoke(f.workspace, action, None).await;
    let digest = digests(&f).await;
    assert_eq!(
        digest.len(),
        1,
        "author timestamps and title edits must not suppress aged state"
    );
    assert!(digest[0].event.content.contains("25h"));
}

#[tokio::test]
#[ignore = "requires isolated Postgres"]
async fn projection_failure_preserves_ingest_ack_and_next_tick_recovers() {
    use crate::handlers::ingest::{ingest_event, IngestAuth};
    let f = Fixture::new().await;
    let root = f.issue("Durable lifecycle retry", false, 25).await;
    alerts(&f).await;
    let (mut policy, policy_event) = f.record(f.workspace, f.autopilot(2, 10)).await;
    let action = json!({"id":"capture","action":"dispatch.assign"});
    f.invoke(f.workspace, action.clone(), None).await;
    policy.revision = 1;
    policy.parent_event = Some(policy_event.id.to_hex());
    policy.request_id = Uuid::new_v4();
    if let DispatchRecordPayload::Autopilot { enabled, .. } = &mut policy.payload {
        *enabled = false;
    }
    let paused = build_dispatch_record(f.workspace, &policy)
        .unwrap()
        .sign_with_keys(&f.owner)
        .unwrap();
    f.save(&paused, f.workspace).await;
    let assignment = f
        .history(f.channel)
        .await
        .into_iter()
        .find(|e| TaskEnvelopeEvent::from_event(e).is_ok())
        .unwrap();
    let mut task = TaskEnvelopeEvent::from_event(&assignment).unwrap().envelope;
    let tenant = TenantContext::resolved(
        f.community,
        format!("dispatch-workflow-{}.test", f.workspace),
    );
    let function = format!("l3_projection_failure_{}", f.workspace.simple());
    // Failure occurs in the real post-store transaction, scoped to this test's
    // tenant and revision inserts. Original task + admission storage still commits.
    // SQL identifiers/data here contain only fixed text and locally generated UUIDs.
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.community_id='{}'::uuid AND NEW.kind=40021 THEN RAISE EXCEPTION 'injected lifecycle projection failure'; END IF; RETURN NEW; END $$", f.community.as_uuid())))
        .execute(&f.pool).await.unwrap();
    let mut previous = assignment.id.to_hex();
    for (status, before, expected) in [
        (
            TaskStatus::PickedUp,
            IssueStatus::Todo,
            IssueStatus::InProgress,
        ),
        (
            TaskStatus::Done,
            IssueStatus::InProgress,
            IssueStatus::InReview,
        ),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE TRIGGER {function} BEFORE INSERT ON events FOR EACH ROW EXECUTE FUNCTION {function}()")))
            .execute(&f.pool).await.unwrap();
        task.status = status;
        let authority = task.authority.as_mut().unwrap();
        authority.assignment_event_id = Some(assignment.id.to_hex());
        authority.previous_event_id = Some(previous.clone());
        let event = build_task_message(f.channel, "Projection retry", &task, None, &[])
            .unwrap()
            .sign_with_keys(&f.agent)
            .unwrap();
        let auth = || IngestAuth::Nip42 {
            pubkey: f.agent.public_key(),
            scopes: vec![buzz_auth::Scope::MessagesWrite],
            channel_ids: None,
            conn_id: Uuid::new_v4(),
            public_scheme: crate::proxy::PublicScheme::Http,
        };
        for _ in 0..2 {
            let ack = ingest_event(&f.state, &tenant, event.clone(), auth()).await;
            assert!(
                ack.is_ok_and(|result| result.accepted),
                "durable task must ACK despite projection failure (including duplicate)"
            );
        }
        assert!(f
            .state
            .db
            .get_event_by_id(f.community, event.id.as_bytes())
            .await
            .unwrap()
            .is_some());
        let fold = preliminary_planner(&f.state, f.community, f.workspace)
            .await
            .unwrap()
            .issues(f.workspace);
        assert_eq!(
            fold.issues
                .values()
                .find(|i| i.root_event == root.id.to_hex())
                .unwrap()
                .status,
            before,
            "injected failure must actually prevent board projection"
        );
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP TRIGGER {function} ON events"
        )))
        .execute(&f.pool)
        .await
        .unwrap();
        let tick = if status == TaskStatus::PickedUp {
            json!({"id":"capture","action":"dispatch.escalate"})
        } else {
            action.clone()
        };
        f.invoke(f.workspace, tick, None).await;
        assert!(
            digests(&f).await.is_empty(),
            "escalation in recovery transaction must see newly observed status age"
        );
        let p = preliminary_planner(&f.state, f.community, f.workspace)
            .await
            .unwrap();
        let fold = p.issues(f.workspace);
        let issue = fold
            .issues
            .values()
            .find(|i| i.root_event == root.id.to_hex())
            .unwrap();
        assert_eq!(
            issue.status, expected,
            "next scheduled Dispatch tick must recover pending lifecycle projection"
        );
        let tip = issue.tip_event.clone();
        f.invoke(f.workspace, action.clone(), None).await;
        let fold = preliminary_planner(&f.state, f.community, f.workspace)
            .await
            .unwrap()
            .issues(f.workspace);
        assert_eq!(
            fold.issues
                .values()
                .find(|i| i.root_event == root.id.to_hex())
                .unwrap()
                .tip_event,
            tip,
            "recovery must be idempotent"
        );
        previous = event.id.to_hex();
    }
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP FUNCTION {function}()")))
        .execute(&f.pool)
        .await
        .unwrap();
}
