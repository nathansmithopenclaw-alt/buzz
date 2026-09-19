use super::*;
use buzz_sdk::{dispatch_record::build_dispatch_record, task_envelope::*};
use nostr::Keys;
use serde_json::{json, Value};
mod recovery_postgres_tests;
mod revision_postgres_tests;
struct Fixture {
    state: Arc<AppState>,
    pool: sqlx::PgPool,
    community: CommunityId,
    workspace: Uuid,
    channel: Uuid,
    owner: Keys,
    agent: Keys,
    project: Uuid,
}
impl Fixture {
    async fn new() -> Self {
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
            buzz_db::migration::run_migrations(&pool).await.unwrap();
        }
        let state = crate::state::tests::test_state_with_database_pool(pool.clone()).await;
        state
            .workflow_engine
            .set_action_sink(Arc::new(crate::workflow_sink::RelayActionSink::new(&state)));
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let workspace = Uuid::new_v4();
        let channel = Uuid::new_v4();
        let owner = Keys::generate();
        let agent = Keys::generate();
        sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
            .bind(community.as_uuid())
            .bind(format!("dispatch-workflow-{}.test", workspace))
            .execute(&pool)
            .await
            .unwrap();
        for key in [&owner, &agent] {
            sqlx::query("INSERT INTO users(community_id,pubkey) VALUES($1,$2)")
                .bind(community.as_uuid())
                .bind(key.public_key().to_bytes().as_slice())
                .execute(&pool)
                .await
                .unwrap();
        }
        for scope in [workspace, channel] {
            sqlx::query("INSERT INTO channels(community_id,id,name,created_by,channel_type,visibility) VALUES($1,$2,'workflow fixture',$3,'stream','private')").bind(community.as_uuid()).bind(scope).bind(owner.public_key().to_bytes().as_slice()).execute(&pool).await.unwrap();
            for (key, role) in [(&owner, "owner"), (&agent, "member")] {
                sqlx::query("INSERT INTO channel_members(community_id,channel_id,pubkey,role) VALUES($1,$2,$3,$4::member_role)").bind(community.as_uuid()).bind(scope).bind(key.public_key().to_bytes().as_slice()).bind(role).execute(&pool).await.unwrap();
            }
        }
        Self {
            state,
            pool,
            community,
            workspace,
            channel,
            owner,
            agent,
            project: Uuid::new_v4(),
        }
    }
    async fn save(&self, event: &Event, channel: Uuid) {
        self.save_observed_hours_ago(event, channel, 0).await;
    }
    // Simulate historical relay observation separately from the author's clock.
    async fn save_observed_hours_ago(&self, event: &Event, channel: Uuid, hours: u64) {
        if requires_dispatch_event_admission(event) {
            self.state
                .db
                .insert_dispatch_with_admission(
                    self.community,
                    event,
                    channel,
                    None,
                    None,
                    |role, now| {
                        build_dispatch_admission(&DispatchAdmission {
                            v: 1,
                            workspace: channel,
                            event_id: event.id.to_hex(),
                            author: event.pubkey.to_hex(),
                            role: if matches!(role, Some("owner" | "admin" | "bot")) {
                                "maintainer".into()
                            } else {
                                "member".into()
                            },
                            observed_at: now - hours * 3600,
                            basis: AdmissionBasis::Admission,
                        })?
                        .custom_created_at(Timestamp::from(now - hours * 3600))
                        .sign_with_keys(&self.state.relay_keypair)
                        .map_err(err)
                    },
                )
                .await
                .unwrap();
        } else {
            self.state
                .db
                .insert_event(self.community, event, Some(channel))
                .await
                .unwrap();
        }
    }
    async fn record(
        &self,
        channel: Uuid,
        payload: DispatchRecordPayload,
    ) -> (DispatchRecord, Event) {
        let record = DispatchRecord {
            v: 1,
            record_id: Uuid::new_v4(),
            revision: 0,
            parent_event: None,
            request_id: Uuid::new_v4(),
            archived: false,
            payload,
        };
        let event = build_dispatch_record(channel, &record)
            .unwrap()
            .sign_with_keys(&self.owner)
            .unwrap();
        self.save(&event, channel).await;
        (record, event)
    }
    async fn issue(&self, title: &str, assigned: bool, age_hours: u64) -> Event {
        let id = Uuid::new_v4();
        let root = IssueRoot {
            v: 1,
            op: "create".into(),
            issue_id: id,
            identifier: format!("TEST-{}", id.simple()),
            title: title.into(),
            body: "Prove the requested work".into(),
            status: IssueStatus::Todo,
            priority: None,
            assignee: assigned.then(|| self.owner.public_key().to_hex()),
            project_id: Some(self.project),
            due: None,
            estimate_minutes: None,
            home_channel_id: Some(self.channel),
            source_event_id: None,
            workflow: "default-v1".into(),
            labels: vec![],
            external_refs: vec![],
            request_id: None,
            import: None,
        };
        let event = build_tracker_issue(self.workspace, &root)
            .unwrap()
            .custom_created_at(Timestamp::from(
                Timestamp::now().as_secs() - age_hours * 3600,
            ))
            .sign_with_keys(&self.owner)
            .unwrap();
        self.save_observed_hours_ago(&event, self.workspace, age_hours)
            .await;
        event
    }
    async fn invoke(&self, channel: Uuid, action: Value, reaction: Option<&Event>) -> Value {
        let capture = action["action"] == "dispatch.create_issue";
        let definition = json!({"name":if capture{"dispatch-capture"}else{"dispatch-test"},"trigger":if capture{json!({"on":"reaction_added","emoji":"📌","filter":format!("trigger_channel_id == \"{channel}\"")})}else{json!({"on":"schedule","interval":"1m"})},"steps":[action]});
        let def: buzz_workflow::WorkflowDef = serde_json::from_value(definition.clone()).unwrap();
        def.validate().unwrap();
        let workflow = self
            .state
            .db
            .create_workflow(
                self.community,
                Some(channel),
                &self.owner.public_key().to_bytes(),
                &def.name,
                &definition.to_string(),
                &[1; 32],
            )
            .await
            .unwrap();
        let ctx = if let Some(reaction) = reaction {
            let stored = self
                .state
                .db
                .get_event_by_id(self.community, reaction.id.as_bytes())
                .await
                .unwrap()
                .unwrap();
            buzz_workflow::build_trigger_context(&stored)
        } else {
            buzz_workflow::executor::TriggerContext {
                channel_id: channel.to_string(),
                ..Default::default()
            }
        };
        let run = self
            .state
            .db
            .create_workflow_run(
                self.community,
                workflow,
                reaction.map(|e| e.id.as_bytes().as_slice()),
                Some(&serde_json::to_value(&ctx).unwrap()),
            )
            .await
            .unwrap();
        let result = buzz_workflow::executor::execute_run(
            &self.state.workflow_engine,
            self.community,
            run,
            &def,
            &ctx,
        )
        .await
        .unwrap();
        result.step_outputs["capture"].clone()
    }
    async fn history(&self, channel: Uuid) -> Vec<Event> {
        self.state
            .db
            .dispatch_workflow_history(self.community, channel)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.event)
            .collect()
    }
    fn autopilot(&self, at_a_time: u32, per_day: u32) -> DispatchRecordPayload {
        DispatchRecordPayload::Autopilot {
            workspace: self.workspace,
            project: self.project,
            agents: vec![self.agent.public_key().to_hex()],
            label: None,
            at_a_time,
            per_day,
            reviewer_required: true,
            enabled: true,
            activation_id: Uuid::new_v4(),
            confirmation_required: true,
            confirmation_event: None,
        }
    }
    async fn assignment_count(&self) -> usize {
        self.history(self.channel)
            .await
            .iter()
            .filter_map(|e| TaskEnvelopeEvent::from_event(e).ok())
            .filter(|t| t.envelope.status == TaskStatus::Assigned)
            .count()
    }
    async fn pin(&self, source: &Event, nonce: &str) -> Event {
        let e = EventBuilder::new(Kind::Custom(7), "📌")
            .tags([
                tag("e", &source.id.to_hex()).unwrap(),
                tag("nonce", nonce).unwrap(),
            ])
            .sign_with_keys(&self.agent)
            .unwrap();
        self.save(&e, self.channel).await;
        e
    }
}

#[tokio::test]
#[ignore = "requires isolated Postgres"]
async fn reaction_capture_is_idempotent_and_requires_current_authorization() {
    let f = Fixture::new().await;
    let source = EventBuilder::new(Kind::Custom(9), "First line title\nLong source body")
        .tags([tag("h", &f.channel.to_string()).unwrap()])
        .sign_with_keys(&f.owner)
        .unwrap();
    f.save(&source, f.channel).await;
    let pin = f.pin(&source, "one").await;
    let action = json!({"id":"capture","action":"dispatch.create_issue"});
    f.invoke(f.channel, action.clone(), Some(&pin)).await;
    assert_eq!(
        f.history(f.workspace)
            .await
            .iter()
            .filter(|e| e.kind.as_u16() == 40020)
            .count(),
        0
    );
    assert!(f.history(f.channel).await.iter().any(
        |e| e.kind.as_u16() == 40023 && e.content.contains("skipped: no authorization record")
    ));
    f.record(
        f.channel,
        DispatchRecordPayload::Capture {
            channel: f.channel,
            workspace: f.workspace,
            project: Some(f.project),
            enabled: true,
        },
    )
    .await;
    f.invoke(f.channel, action.clone(), Some(&pin)).await;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let another = f.pin(&source, "two").await;
    f.invoke(f.channel, action.clone(), Some(&another)).await;
    let history = f.history(f.workspace).await;
    let roots = history
        .iter()
        .filter(|e| e.kind.as_u16() == 40020)
        .collect::<Vec<_>>();
    assert_eq!(
        roots.len(),
        1,
        "duplicate pin must not create a second issue"
    );
    let root: IssueRoot = serde_json::from_str(&roots[0].content).unwrap();
    assert_eq!(root.title, "First line title");
    assert_eq!(root.status, IssueStatus::Triage);
    assert_eq!(root.home_channel_id, Some(f.channel));
    assert!(root.body.contains(&source.id.to_hex()));
    sqlx::query("UPDATE channel_members SET role='member' WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3").bind(f.community.as_uuid()).bind(f.channel).bind(f.owner.public_key().to_bytes().as_slice()).execute(&f.pool).await.unwrap();
    f.invoke(f.channel, action, Some(&another)).await;
    assert!(f
        .history(f.channel)
        .await
        .iter()
        .any(|e| e.kind.as_u16() == 40023 && e.content.contains("signer not authorized")));
}

#[tokio::test]
#[ignore = "requires isolated Postgres"]
async fn scheduled_escalation_coalesces_digest_and_never_assigns_twice() {
    let f = Fixture::new().await;
    f.issue("Unowned one", false, 25).await;
    f.issue("Unowned two", false, 30).await;
    f.issue("Already owned", true, 30).await;
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
    let action = json!({"id":"capture","action":"dispatch.escalate"});
    f.invoke(f.workspace, action.clone(), None).await;
    f.invoke(f.workspace, action, None).await;
    let query = buzz_db::EventQuery {
        community_id: f.community,
        kinds: Some(vec![9]),
        channel_id: Some(f.channel),
        limit: Some(100),
        ..buzz_db::EventQuery::for_community(f.community)
    };
    let messages = f.state.db.query_events(&query).await.unwrap();
    let digests = messages
        .iter()
        .filter(|e| {
            e.event
                .content
                .starts_with("Work that still needs an owner:")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        digests.len(),
        1,
        "one digest per channel across repeated schedule ticks"
    );
    assert!(digests[0].event.content.contains("Unowned one"));
    assert!(digests[0].event.content.contains("Unowned two"));
    assert!(!digests[0].event.content.contains("Already owned"));
    assert_eq!(f.assignment_count().await, 0);
    assert_eq!(
        f.history(f.workspace)
            .await
            .iter()
            .filter(|e| e.kind.as_u16() == 40023)
            .count(),
        2
    );
}

#[tokio::test]
#[ignore = "requires isolated Postgres"]
async fn autopilot_assignment_enforces_concurrency_and_daily_cap() {
    let action = json!({"id":"capture","action":"dispatch.assign"});
    let f = Fixture::new().await;
    for n in 0..4 {
        f.issue(&format!("Ready {n}"), true, 0).await;
    }
    f.record(f.workspace, f.autopilot(2, 10)).await;
    f.invoke(f.workspace, action.clone(), None).await;
    f.invoke(f.workspace, action.clone(), None).await;
    assert_eq!(
        f.assignment_count().await,
        2,
        "at-a-time cap must hold across ticks"
    );
    let day = Fixture::new().await;
    for n in 0..4 {
        day.issue(&format!("Daily {n}"), true, 0).await;
    }
    day.record(day.workspace, day.autopilot(8, 1)).await;
    day.invoke(day.workspace, action.clone(), None).await;
    day.invoke(day.workspace, action, None).await;
    assert_eq!(
        day.assignment_count().await,
        1,
        "per-day cap must prevent a second assignment"
    );
    let history = f.history(f.workspace).await;
    for task in f
        .history(f.channel)
        .await
        .iter()
        .filter_map(|e| TaskEnvelopeEvent::from_event(e).ok())
    {
        assert_eq!(task.envelope.v, 3);
        assert_eq!(
            task.envelope.authority.unwrap().reviewer,
            Some(f.owner.public_key().to_hex())
        );
        assert!(
            history
                .iter()
                .filter_map(|e| serde_json::from_str::<IssueRevision>(&e.content).ok())
                .any(|r| r
                    .external_refs
                    .add
                    .iter()
                    .any(|r| r.kind == "task_event" && r.reference == task.event_id)),
            "task must already be linked in committed issue history"
        );
    }
}

#[tokio::test]
#[ignore = "requires isolated Postgres"]
async fn autopilot_first_hour_requires_confirmation_and_pause_preserves_history() {
    let f = Fixture::new().await;
    for n in 0..6 {
        f.issue(&format!("First hour {n}"), true, 0).await;
    }
    let (mut record, mut head) = f.record(f.workspace, f.autopilot(10, 10)).await;
    let action = json!({"id":"capture","action":"dispatch.assign"});
    let output = f.invoke(f.workspace, action.clone(), None).await;
    assert_eq!(f.assignment_count().await, 3);
    assert_eq!(output["confirmation_required"], true);
    record.revision += 1;
    record.parent_event = Some(head.id.to_hex());
    record.request_id = Uuid::new_v4();
    if let DispatchRecordPayload::Autopilot {
        confirmation_required,
        confirmation_event,
        ..
    } = &mut record.payload
    {
        *confirmation_required = false;
        *confirmation_event = output["confirmation_event"].as_str().map(str::to_owned);
    }
    head = build_dispatch_record(f.workspace, &record)
        .unwrap()
        .sign_with_keys(&f.owner)
        .unwrap();
    f.save(&head, f.workspace).await;
    f.invoke(f.workspace, action.clone(), None).await;
    assert_eq!(f.assignment_count().await, 6);
    f.issue("After pause", true, 0).await;
    record.revision += 1;
    record.parent_event = Some(head.id.to_hex());
    record.request_id = Uuid::new_v4();
    if let DispatchRecordPayload::Autopilot { enabled, .. } = &mut record.payload {
        *enabled = false;
    }
    let head = build_dispatch_record(f.workspace, &record)
        .unwrap()
        .sign_with_keys(&f.owner)
        .unwrap();
    f.save(&head, f.workspace).await;
    f.invoke(f.workspace, action, None).await;
    assert_eq!(
        f.assignment_count().await,
        6,
        "pause must retain history without assigning more work"
    );
}

#[tokio::test]
#[ignore = "requires isolated Postgres"]
async fn accepted_task_updates_move_board_without_separate_tracker_command() {
    use crate::handlers::ingest::{ingest_event, IngestAuth};
    let f = Fixture::new().await;
    let root = f.issue("Lifecycle", true, 0).await;
    f.record(f.workspace, f.autopilot(2, 10)).await;
    f.invoke(
        f.workspace,
        json!({"id":"capture","action":"dispatch.assign"}),
        None,
    )
    .await;
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
    let mut previous = assignment.id.to_hex();
    for (status, expected) in [
        (TaskStatus::PickedUp, IssueStatus::InProgress),
        (TaskStatus::Done, IssueStatus::InReview),
    ] {
        task.status = status;
        let authority = task.authority.as_mut().unwrap();
        authority.assignment_event_id = Some(assignment.id.to_hex());
        authority.previous_event_id = Some(previous.clone());
        let event = build_task_message(f.channel, "Execution update", &task, None, &[])
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
        let result = ingest_event(&f.state, &tenant, event.clone(), auth())
            .await
            .unwrap();
        assert!(result.accepted);
        let retry = ingest_event(&f.state, &tenant, event.clone(), auth())
            .await
            .unwrap();
        assert!(retry.accepted);
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
            "accepted envelope lifecycle must automatically update the board"
        );
        assert_eq!(
            issue.assignee,
            Some(f.owner.public_key().to_hex()),
            "agent execution must preserve human accountability"
        );
        previous = event.id.to_hex();
    }
}

#[tokio::test]
#[ignore = "requires isolated Postgres"]
async fn concurrent_ticks_and_invalid_confirmation_cannot_exceed_first_hour_cap() {
    let f = Fixture::new().await;
    for n in 0..8 {
        f.issue(&format!("Concurrent {n}"), true, 0).await;
    }
    let (mut record, head) = f.record(f.workspace, f.autopilot(10, 10)).await;
    record.revision = 1;
    record.parent_event = Some(head.id.to_hex());
    record.request_id = Uuid::new_v4();
    if let DispatchRecordPayload::Autopilot {
        confirmation_required,
        confirmation_event,
        ..
    } = &mut record.payload
    {
        *confirmation_required = false;
        *confirmation_event = Some("a".repeat(64));
    }
    let premature = build_dispatch_record(f.workspace, &record)
        .unwrap()
        .sign_with_keys(&f.owner)
        .unwrap();
    f.save(&premature, f.workspace).await;
    let action = json!({"id":"capture","action":"dispatch.assign"});
    tokio::join!(
        f.invoke(f.workspace, action.clone(), None),
        f.invoke(f.workspace, action.clone(), None)
    );
    f.invoke(f.workspace, action, None).await;
    assert_eq!(
        f.assignment_count().await,
        3,
        "unbound confirmation and concurrent ticks cannot bypass first-hour cap"
    );
}

#[tokio::test]
#[ignore = "requires isolated Postgres"]
async fn backdated_autopilot_still_warms_up_from_relay_admission_and_catalog_is_closed() {
    let f = Fixture::new().await;
    f.state.db.validate_deletion_catalog().await.unwrap();
    f.state
        .db
        .validate_deletion_serving_catalog()
        .await
        .unwrap();
    for n in 0..5 {
        f.issue(&format!("Backdated {n}"), true, 0).await;
    }
    let record = DispatchRecord {
        v: 1,
        record_id: Uuid::new_v4(),
        revision: 0,
        parent_event: None,
        request_id: Uuid::new_v4(),
        archived: false,
        payload: f.autopilot(10, 10),
    };
    let event = build_dispatch_record(f.workspace, &record)
        .unwrap()
        .custom_created_at(Timestamp::from(Timestamp::now().as_secs() - 86400))
        .sign_with_keys(&f.owner)
        .unwrap();
    f.save(&event, f.workspace).await;
    f.invoke(
        f.workspace,
        json!({"id":"capture","action":"dispatch.assign"}),
        None,
    )
    .await;
    assert_eq!(
        f.assignment_count().await,
        3,
        "author clock cannot bypass relay first-hour cap"
    );
}
