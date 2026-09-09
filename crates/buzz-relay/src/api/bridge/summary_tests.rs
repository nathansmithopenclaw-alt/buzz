//! Summary authorization tests through the production HTTP query handler.
use super::*;
use buzz_core::summary::{SummaryAudience, SummaryDefinition, SummaryReport};
use buzz_core::CommunityId;
use nostr::{EventBuilder, Keys, Kind, Tag};
use std::collections::BTreeSet;
use uuid::Uuid;

struct Fixture {
    state: Arc<AppState>,
    tenant: TenantContext,
    pool: sqlx::PgPool,
    people: Vec<Keys>,
    reporter: Keys,
    unrelated: Keys,
    channel: Uuid,
}
impl Fixture {
    async fn new() -> Self {
        let mut state = super::postgres_tests::bridge_handler_test_state()
            .await
            .expect("isolated Postgres and Redis required");
        Arc::get_mut(&mut Arc::get_mut(&mut state).unwrap().config)
            .unwrap()
            .require_relay_membership = true;
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let id = Uuid::new_v4();
        let host = format!("summary-{}.test", id);
        sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
            .bind(id)
            .bind(&host)
            .execute(&pool)
            .await
            .unwrap();
        let tenant = TenantContext::resolved(CommunityId::from_uuid(id), host);
        let people = vec![Keys::generate(), Keys::generate(), Keys::generate()];
        let reporter = Keys::generate();
        let unrelated = Keys::generate();
        for key in people.iter().chain([&reporter, &unrelated]) {
            let agent = key.public_key() == reporter.public_key()
                || key.public_key() == unrelated.public_key();
            sqlx::query("INSERT INTO users(community_id,pubkey,agent_owner_pubkey,agent_type) VALUES($1,$2,$3,$4)")
                .bind(id).bind(key.public_key().to_bytes().as_slice()).bind(if agent{Some(people[0].public_key().to_bytes().to_vec())}else{None})
                .bind(if agent{Some("agent")}else{None}).execute(&pool).await.unwrap();
            sqlx::query(
                "INSERT INTO relay_members(community_id,pubkey,role) VALUES($1,$2,'member')",
            )
            .bind(id)
            .bind(key.public_key().to_hex())
            .execute(&pool)
            .await
            .unwrap();
        }
        let channel = Uuid::new_v4();
        sqlx::query("INSERT INTO channels(community_id,id,name,created_by) VALUES($1,$2,'Summary source',$3)")
            .bind(id).bind(channel).bind(people[0].public_key().to_bytes().as_slice()).execute(&pool).await.unwrap();
        for key in &people {
            sqlx::query(
                "INSERT INTO channel_members(community_id,channel_id,pubkey) VALUES($1,$2,$3)",
            )
            .bind(id)
            .bind(channel)
            .bind(key.public_key().to_bytes().as_slice())
            .execute(&pool)
            .await
            .unwrap();
        }
        Self {
            state,
            tenant,
            pool,
            people,
            reporter,
            unrelated,
            channel,
        }
    }
    async fn config(&self, id: &str, audience: SummaryAudience, at: u64) -> nostr::Event {
        let config = SummaryDefinition {
            v: 2,
            summary_id: id.into(),
            name: "Shared report".into(),
            reporter_pubkey: self.reporter.public_key().to_hex(),
            audience,
            read_delegation: true,
            channels: vec![self.channel],
            dms: vec![],
            exclude: vec![],
            auto_include: false,
            schedule: None,
            timezone: "UTC".into(),
            paused: false,
            archived: false,
        };
        let event = EventBuilder::new(Kind::Custom(30623), serde_json::to_string(&config).unwrap())
            .tags([
                Tag::parse(["d", &buzz_core::summary::config_d_tag(id).unwrap()]).unwrap(),
                Tag::parse(["p", &self.people[0].public_key().to_hex()]).unwrap(),
                Tag::parse(["p", &self.reporter.public_key().to_hex()]).unwrap(),
            ])
            .allow_self_tagging()
            .custom_created_at(nostr::Timestamp::from(at))
            .sign_with_keys(&self.people[0])
            .unwrap();
        crate::summary::validate_event(&self.state, self.tenant.community(), &event)
            .await
            .unwrap();
        self.state
            .db
            .replace_summary_definition(
                self.tenant.community(),
                &event,
                &buzz_core::summary::config_d_tag(id).unwrap(),
            )
            .await
            .unwrap();
        event
    }
    async fn report(
        &self,
        id: &str,
        config: &nostr::Event,
        audience: BTreeSet<String>,
    ) -> nostr::Event {
        let (run, _) = self
            .state
            .db
            .claim_summary_run(
                self.tenant.community(),
                &self.people[0].public_key().to_bytes(),
                id,
                config.id.as_bytes(),
                &self.reporter.public_key().to_bytes(),
                &audience,
            )
            .await
            .unwrap();
        let report = SummaryReport {
            v: 2,
            owner_pubkey: self.people[0].public_key().to_hex(),
            summary_id: id.into(),
            run_id: run.run_id,
            config_event_id: config.id.to_hex(),
            title: "Private report title".into(),
            markdown: "No new activity.".into(),
            status: serde_json::json!({}),
            sources: vec![],
            since: run.started_at.timestamp(),
            until: run.started_at.timestamp(),
            watermarks: vec![],
        };
        let event = buzz_sdk::build_summary_report(
            &report,
            &self.reporter.public_key().to_hex(),
            &audience,
        )
        .unwrap()
        .sign_with_keys(&self.reporter)
        .unwrap();
        crate::summary::validate_event(&self.state, self.tenant.community(), &event)
            .await
            .unwrap();
        self.state
            .db
            .publish_summary_report(self.tenant.community(), &event)
            .await
            .unwrap();
        event
    }
    async fn query(
        &self,
        key: &Keys,
        filter: Value,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        query_events_authed(
            &self.state,
            &self.tenant,
            &HeaderMap::new(),
            &serde_json::to_vec(&vec![filter]).unwrap(),
            key.public_key(),
            [7; 32],
            None,
        )
        .await
    }
    async fn can_read(&self, key: &Keys, event: &nostr::Event) -> bool {
        let response = self
            .query(
                key,
                serde_json::json!({"ids":[event.id.to_hex()],"#p":[key.public_key().to_hex()]}),
            )
            .await
            .unwrap()
            .0;
        response
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == event.id.to_hex())
    }
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn summary_direct_id_audience_and_live_revocation() {
    let f = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    let at = nostr::Timestamp::now().as_secs();
    let config = f
        .config(
            &id,
            SummaryAudience::Selected {
                people: vec![f.people[1].public_key().to_hex()],
            },
            at,
        )
        .await;
    let report = f
        .report(
            &id,
            &config,
            [
                f.people[0].public_key().to_hex(),
                f.people[1].public_key().to_hex(),
            ]
            .into(),
        )
        .await;
    assert!(f.can_read(&f.people[0], &report).await);
    assert!(f.can_read(&f.people[1], &report).await);
    assert!(!f.can_read(&f.people[2], &report).await);
    assert!(!f.can_read(&f.unrelated, &report).await);
    assert!(f.can_read(&f.reporter, &report).await);
    assert!(
        !f.can_read(&f.people[1], &config).await,
        "recipient must not see private config/source metadata"
    );
    f.config(&id, SummaryAudience::OnlyMe, at + 1).await;
    assert!(
        !f.can_read(&f.people[1], &report).await,
        "immutable p tag cannot defeat live narrowing"
    );
    assert!(f.can_read(&f.people[0], &report).await);
    assert!(
        !crate::handlers::req::event_visible_to_reader_with_policy(
            &f.state,
            f.tenant.community(),
            &report,
            &f.people[1].public_key().to_bytes()
        )
        .await
    );
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn summary_community_snapshot_does_not_grant_history_to_new_people() {
    let f = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    let config = f
        .config(
            &id,
            SummaryAudience::Community,
            nostr::Timestamp::now().as_secs(),
        )
        .await;
    let report = f
        .report(
            &id,
            &config,
            f.people.iter().map(|k| k.public_key().to_hex()).collect(),
        )
        .await;
    for person in &f.people {
        assert!(f.can_read(person, &report).await);
    }
    assert!(!f.can_read(&f.unrelated, &report).await);
    let newcomer = Keys::generate();
    sqlx::query("INSERT INTO users(community_id,pubkey) VALUES($1,$2)")
        .bind(f.tenant.community().as_uuid())
        .bind(newcomer.public_key().to_bytes().as_slice())
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO relay_members(community_id,pubkey,role) VALUES($1,$2,'member')")
        .bind(f.tenant.community().as_uuid())
        .bind(newcomer.public_key().to_hex())
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(!f.can_read(&newcomer, &report).await);
    sqlx::query("DELETE FROM relay_members WHERE community_id=$1 AND pubkey=$2")
        .bind(f.tenant.community().as_uuid())
        .bind(f.people[1].public_key().to_hex())
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(
        !crate::summary::reader_allowed(
            &f.state,
            f.tenant.community(),
            &report,
            &f.people[1].public_key().to_bytes()
        )
        .await
    );
}

impl Fixture {
    async fn count_and_live(&self, key: &Keys, event: &nostr::Event) -> (u64, bool) {
        use crate::connection::{AuthState, ConnectionState};
        use std::{collections::HashMap, sync::atomic::AtomicU8};
        use tokio::sync::{mpsc, Mutex, RwLock};
        use tokio_util::sync::CancellationToken;
        let (tx, mut rx) = mpsc::channel(8);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
        let conn = Arc::new(ConnectionState {
            conn_id: Uuid::new_v4(),
            tenant: self.tenant.clone(),
            remote_addr: "127.0.0.1:1234".parse().unwrap(),
            public_scheme: crate::proxy::PublicScheme::Http,
            auth_state: RwLock::new(AuthState::Authenticated(buzz_auth::AuthContext {
                pubkey: key.public_key(),
                scopes: vec![],
                channel_ids: None,
                auth_method: buzz_auth::AuthMethod::Nip42,
                agent_owner_pubkey: None,
            })),
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
            send_tx: tx.clone(),
            ctrl_tx: ctrl_tx.clone(),
            cancel: CancellationToken::new(),
            backpressure_count: Arc::new(AtomicU8::new(0)),
            grace_limit: 3,
        });
        self.state.conn_manager.register(
            conn.conn_id,
            tx,
            ctrl_tx,
            None,
            conn.cancel.clone(),
            self.tenant.community(),
            conn.backpressure_count.clone(),
            conn.subscriptions.clone(),
            3,
        );
        self.state
            .conn_manager
            .set_authenticated_pubkey(conn.conn_id, key.public_key().to_bytes().to_vec());
        let filter: nostr::Filter = serde_json::from_value(serde_json::json!({"kinds":[40030],"ids":[event.id.to_hex()],"#p":[key.public_key().to_hex()]})).unwrap();
        crate::handlers::count::handle_count(
            "summary-count".into(),
            vec![filter],
            conn.clone(),
            self.state.clone(),
        )
        .await;
        let frame = rx.try_recv().unwrap().into_text().unwrap();
        let value: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(value[0], "COUNT", "{value}");
        let http_count = count_events_authed(&self.state, &self.tenant, &HeaderMap::new(),
            &serde_json::to_vec(&serde_json::json!([{"kinds":[40030],"ids":[event.id.to_hex()],"#p":[key.public_key().to_hex()]}])).unwrap(),
            key.public_key(), [8;32], None).await.unwrap().0;
        assert_eq!(
            http_count["count"], value[2]["count"],
            "HTTP and WebSocket counts must enforce the same live policy"
        );
        let live = crate::handlers::event::filter_fanout_by_access(
            &self.state,
            self.tenant.community(),
            &buzz_core::StoredEvent::new(event.clone(), None),
            vec![(conn.conn_id, "summary-live".into())],
            None,
        )
        .await;
        (value[2]["count"].as_u64().unwrap(), !live.is_empty())
    }
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn summary_count_and_fanout_reauthorize_frozen_private_sources() {
    let f = Fixture::new().await;
    sqlx::query("UPDATE channels SET visibility='private' WHERE community_id=$1 AND id=$2")
        .bind(f.tenant.community().as_uuid())
        .bind(f.channel)
        .execute(&f.pool)
        .await
        .unwrap();
    let id = Uuid::new_v4().to_string();
    let config = f
        .config(
            &id,
            SummaryAudience::Selected {
                people: vec![f.people[1].public_key().to_hex()],
            },
            nostr::Timestamp::now().as_secs(),
        )
        .await;
    // Empty model-reported sources must not bypass the frozen private source policy.
    let report = f
        .report(
            &id,
            &config,
            [
                f.people[0].public_key().to_hex(),
                f.people[1].public_key().to_hex(),
            ]
            .into(),
        )
        .await;
    assert_eq!(f.count_and_live(&f.people[1], &report).await, (1, true));
    assert_eq!(f.count_and_live(&f.people[2], &report).await, (0, false));
    assert_eq!(f.count_and_live(&f.unrelated, &report).await, (0, false));
    sqlx::query(
        "DELETE FROM channel_members WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3",
    )
    .bind(f.tenant.community().as_uuid())
    .bind(f.channel)
    .bind(f.people[1].public_key().to_bytes().as_slice())
    .execute(&f.pool)
    .await
    .unwrap();
    assert!(!f.can_read(&f.people[1], &report).await);
    assert_eq!(f.count_and_live(&f.people[1], &report).await, (0, false));
    assert_eq!(f.count_and_live(&f.people[0], &report).await, (1, true));
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn summary_delegated_read_and_progress_require_exact_active_run() {
    let f = Fixture::new().await;
    sqlx::query("UPDATE channels SET visibility='private',channel_type='dm' WHERE community_id=$1 AND id=$2").bind(f.tenant.community().as_uuid()).bind(f.channel).execute(&f.pool).await.unwrap();
    let id = Uuid::new_v4().to_string();
    let at = nostr::Timestamp::now().as_secs();
    let config = f.config(&id, SummaryAudience::OnlyMe, at).await;
    let source = EventBuilder::new(Kind::Custom(9), "private transcript")
        .tags([Tag::parse(["h", &f.channel.to_string()]).unwrap()])
        .sign_with_keys(&f.people[0])
        .unwrap();
    f.state
        .db
        .insert_event(f.tenant.community(), &source, Some(f.channel))
        .await
        .unwrap();
    let start = serde_json::json!({"summary_operation":"start","summary_id":id});
    assert!(f.query(&f.people[1], start.clone()).await.is_err());
    let run = f.query(&f.people[0], start).await.unwrap().0["run"]["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let read = serde_json::json!({"summary_owner":f.people[0].public_key().to_hex(),"summary_id":id,"summary_run":run,"#h":[f.channel.to_string()]});
    let returned = f.query(&f.reporter, read.clone()).await.unwrap().0;
    assert_eq!(returned["events"][0]["id"], source.id.to_hex());
    assert!(f.query(&f.unrelated, read.clone()).await.is_err());
    let progress = serde_json::json!({"summary_operation":"progress","owner":f.people[0].public_key().to_hex(),"summary_id":id,"run_id":run,"state":"needs_approval","message":"Awaiting owner"});
    assert!(f.query(&f.people[1], progress.clone()).await.is_err());
    let _ = f.query(&f.reporter, progress.clone()).await.unwrap();
    assert!(f.query(&f.reporter, read.clone()).await.is_err());
    let mut resume = progress;
    resume["state"] = serde_json::json!("running");
    let _ = f.query(&f.reporter, resume).await.unwrap();
    assert!(f.query(&f.reporter, read.clone()).await.is_ok());
    f.config(&id, SummaryAudience::OnlyMe, at + 1).await;
    assert!(
        f.query(&f.reporter, read).await.is_err(),
        "edited config must invalidate the run's grant"
    );
    assert!(f
        .state
        .db
        .summary_cursors(
            f.tenant.community(),
            &f.people[0].public_key().to_bytes(),
            &id
        )
        .await
        .unwrap()
        .is_empty());
    assert_ne!(config.id, source.id);
}

#[tokio::test]
#[ignore = "requires isolated Postgres and Redis"]
async fn summary_report_archive_delete_boundaries() {
    let f = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    let audience: BTreeSet<_> = [
        f.people[0].public_key().to_hex(),
        f.people[1].public_key().to_hex(),
    ]
    .into();
    let config = f
        .config(
            &id,
            SummaryAudience::Selected {
                people: vec![f.people[1].public_key().to_hex()],
            },
            nostr::Timestamp::now().as_secs(),
        )
        .await;
    let report = f.report(&id, &config, audience).await;
    let list = |archived| serde_json::json!({"summary_operation":"reports_list","filter":{"archived":archived}});
    let action = |op| serde_json::json!({"summary_operation":op,"event_ids":[report.id.to_hex()]});
    assert_eq!(
        f.query(&f.people[1], list(false)).await.unwrap().0["events"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(f
        .query(&f.people[2], action("reports_archive"))
        .await
        .is_err());
    assert!(f
        .query(&f.people[1], action("reports_delete"))
        .await
        .is_err());
    let _ = f
        .query(&f.people[1], action("reports_archive"))
        .await
        .unwrap();
    assert_eq!(
        f.query(&f.people[1], list(false)).await.unwrap().0["events"],
        serde_json::json!([])
    );
    assert_eq!(
        f.query(&f.people[1], list(true)).await.unwrap().0["events"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        f.query(&f.people[0], list(false)).await.unwrap().0["events"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let _ = f
        .query(&f.people[1], action("reports_restore"))
        .await
        .unwrap();
    assert_eq!(
        f.query(&f.people[1], list(true)).await.unwrap().0["events"],
        serde_json::json!([])
    );
    assert_eq!(f.query(&f.people[0], serde_json::json!({"summary_operation":"reports_list","filter":{"archived":false,"query":"does-not-exist"}})).await.unwrap().0["events"],serde_json::json!([]));
    let _ = f
        .query(&f.people[0], action("reports_delete"))
        .await
        .unwrap();
    assert!(!f.can_read(&f.people[0], &report).await);
    assert!(!f.can_read(&f.people[1], &report).await);
    assert_eq!(
        f.query(&f.people[0], list(false)).await.unwrap().0["events"],
        serde_json::json!([])
    );
    // Publication retry cannot restore a deleted report or reopen its run.
    f.state
        .db
        .publish_summary_report(f.tenant.community(), &report)
        .await
        .unwrap();
    assert!(!f.can_read(&f.people[0], &report).await);
    let history = f
        .query(
            &f.people[0],
            serde_json::json!({"summary_operation":"history","summary_id":id}),
        )
        .await
        .unwrap()
        .0;
    assert_eq!(history["runs"][0]["state"], "published");
    assert_eq!(history["runs"][0]["report_event_id"], report.id.to_hex());
}

#[tokio::test]
#[ignore = "requires isolated Postgres and Redis"]
async fn summary_report_pages_and_archive_revocation() {
    let f = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    let at = nostr::Timestamp::now().as_secs();
    let people = vec![f.people[1].public_key().to_hex()];
    let audience: BTreeSet<_> = [f.people[0].public_key().to_hex(), people[0].clone()].into();
    let config = f
        .config(&id, SummaryAudience::Selected { people }, at)
        .await;
    for _ in 0..52 {
        f.report(&id, &config, audience.clone()).await;
    }
    let list = serde_json::json!({"summary_operation":"reports_list","filter":{"archived":false,"summary_id":id,"query":"Private report title","reporter":f.reporter.public_key().to_hex()}});
    let first = f.query(&f.people[1], list.clone()).await.unwrap().0;
    assert_eq!(first["events"].as_array().unwrap().len(), 50);
    let mut next = list.clone();
    next["filter"]["before_time"] = first["next"]["before_time"].clone();
    next["filter"]["before_id"] = first["next"]["before_id"].clone();
    let last = f.query(&f.people[1], next).await.unwrap().0;
    assert_eq!(last["events"].as_array().unwrap().len(), 2);
    assert!(last["next"].is_null());
    for row in last["events"].as_array().unwrap() {
        assert!(!first["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == row["id"]));
    }
    let selected: Vec<_> = first["events"]
        .as_array()
        .unwrap()
        .iter()
        .take(2)
        .map(|e| e["id"].clone())
        .collect();
    let _ = f
        .query(
            &f.people[1],
            serde_json::json!({"summary_operation":"reports_archive","event_ids":selected}),
        )
        .await
        .unwrap();
    let archived =
        serde_json::json!({"summary_operation":"reports_list","filter":{"archived":true}});
    assert_eq!(
        f.query(&f.people[1], archived.clone()).await.unwrap().0["events"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    f.config(&id, SummaryAudience::OnlyMe, at + 1).await;
    let withdrawn = f.query(&f.people[1], archived).await.unwrap().0;
    assert_eq!(withdrawn["events"], serde_json::json!([]));
    assert!(withdrawn["next"].is_null());
    assert!(f
        .query(
            &f.people[1],
            serde_json::json!({"summary_operation":"reports_restore","event_ids":selected})
        )
        .await
        .is_err());
    let visible = f.query(&f.people[0], list).await.unwrap().0;
    assert_eq!(visible["events"].as_array().unwrap().len(), 50);
}
