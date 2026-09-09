//! PostgreSQL acceptance tests exercise the production claim/read/commit methods.
use super::*;
use buzz_core::{
    summary::{SummaryAudience, SummaryCursor, SummaryDefinition, SummaryReport},
    CommunityId,
};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use sqlx::PgPool;
use std::collections::BTreeSet;
use uuid::Uuid;

struct Fixture {
    db: crate::Db,
    community: CommunityId,
    people: Vec<Keys>,
    reporter: Keys,
    unrelated: Keys,
    channel: Uuid,
}
impl Fixture {
    async fn new() -> Self {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let db = crate::Db::from_pool(pool);
        let community = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
            .bind(community.as_uuid())
            .bind(format!("summary-{}.test", Uuid::new_v4()))
            .execute(&db.pool)
            .await
            .unwrap();
        let people = vec![Keys::generate(), Keys::generate(), Keys::generate()];
        let reporter = Keys::generate();
        let unrelated = Keys::generate();
        for key in people.iter().chain([&reporter, &unrelated]) {
            sqlx::query("INSERT INTO users(community_id,pubkey,agent_owner_pubkey,agent_type) VALUES($1,$2,$3,$4)")
                .bind(community.as_uuid()).bind(key.public_key().to_bytes().as_slice())
                .bind(if key.public_key()==reporter.public_key() || key.public_key()==unrelated.public_key() {Some(people[0].public_key().to_bytes().to_vec())} else {None})
                .bind(if key.public_key()==reporter.public_key() || key.public_key()==unrelated.public_key() {Some("agent")} else {None})
                .execute(&db.pool).await.unwrap();
            sqlx::query(
                "INSERT INTO relay_members(community_id,pubkey,role) VALUES($1,$2,'member')",
            )
            .bind(community.as_uuid())
            .bind(key.public_key().to_hex())
            .execute(&db.pool)
            .await
            .unwrap();
        }
        let channel = Uuid::new_v4();
        sqlx::query("INSERT INTO channels(community_id,id,name,created_by,channel_type,visibility) VALUES($1,$2,'private fixture',$3,'dm','private')")
            .bind(community.as_uuid()).bind(channel).bind(people[0].public_key().to_bytes().as_slice()).execute(&db.pool).await.unwrap();
        for key in &people {
            sqlx::query(
                "INSERT INTO channel_members(community_id,channel_id,pubkey) VALUES($1,$2,$3)",
            )
            .bind(community.as_uuid())
            .bind(channel)
            .bind(key.public_key().to_bytes().as_slice())
            .execute(&db.pool)
            .await
            .unwrap();
        }
        Self {
            db,
            community,
            people,
            reporter,
            unrelated,
            channel,
        }
    }
    fn owner(&self) -> Vec<u8> {
        self.people[0].public_key().to_bytes().to_vec()
    }
    async fn config(&self, id: &str) -> Event {
        let config = SummaryDefinition {
            v: 2,
            summary_id: id.into(),
            name: "Support".into(),
            reporter_pubkey: self.reporter.public_key().to_hex(),
            audience: SummaryAudience::Selected {
                people: vec![self.people[1].public_key().to_hex()],
            },
            read_delegation: true,
            channels: vec![],
            dms: vec![self.channel],
            exclude: vec![],
            auto_include: false,
            schedule: None,
            timezone: "UTC".into(),
            paused: false,
            archived: false,
        };
        let event = EventBuilder::new(Kind::Custom(30623), serde_json::to_string(&config).unwrap())
            .tags([Tag::parse(["d", &buzz_core::summary::config_d_tag(id).unwrap()]).unwrap()])
            .sign_with_keys(&self.people[0])
            .unwrap();
        self.db
            .replace_parameterized_event(
                self.community,
                &event,
                &buzz_core::summary::config_d_tag(id).unwrap(),
                None,
            )
            .await
            .unwrap();
        event
    }
    async fn claim(&self, id: &str, config: &Event) -> (summary_runs::SummaryRun, bool) {
        self.db
            .claim_summary_run(
                self.community,
                &self.owner(),
                id,
                config.id.as_bytes(),
                &self.reporter.public_key().to_bytes(),
                &[
                    self.people[0].public_key().to_hex(),
                    self.people[1].public_key().to_hex(),
                ]
                .into(),
            )
            .await
            .unwrap()
    }
    fn source(&self) -> Event {
        EventBuilder::new(Kind::Custom(40001), "Observed support activity")
            .tags([Tag::parse(["h", &self.channel.to_string()]).unwrap()])
            .sign_with_keys(&self.people[0])
            .unwrap()
    }
    fn report(&self, id: &str, run: &summary_runs::SummaryRun, source: &Event) -> Event {
        let content = SummaryReport {
            v: 2,
            owner_pubkey: hex::encode(self.owner()),
            summary_id: id.into(),
            run_id: run.run_id,
            config_event_id: hex::encode(&run.config_event_id),
            title: "Support".into(),
            markdown: "Observed support activity.".into(),
            status: serde_json::json!({}),
            sources: vec![self.channel],
            since: source.created_at.as_secs() as i64,
            until: run.started_at.timestamp(),
            watermarks: vec![SummaryCursor {
                channel_id: self.channel,
                event_id: source.id.to_hex(),
                created_at: source.created_at.as_secs() as i64,
            }],
        };
        let mut recipients: BTreeSet<String> =
            serde_json::from_value(run.audience.clone()).unwrap();
        recipients.insert(self.reporter.public_key().to_hex());
        EventBuilder::new(
            Kind::Custom(40030),
            serde_json::to_string(&content).unwrap(),
        )
        .tags(recipients.iter().map(|p| Tag::parse(["p", p]).unwrap()))
        .allow_self_tagging()
        .sign_with_keys(&self.reporter)
        .unwrap()
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn summary_concurrent_claims_and_reporter_independent_cursors() {
    let f = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    let config = f.config(&id).await;
    let ((a, created_a), (b, created_b)) =
        tokio::join!(f.claim(&id, &config), f.claim(&id, &config));
    assert_eq!(a.run_id, b.run_id);
    assert_ne!(created_a, created_b);
    let other = Uuid::new_v4().to_string();
    let other_config = f.config(&other).await;
    let (other_run, created) = f.claim(&other, &other_config).await;
    assert!(created);
    assert_ne!(a.run_id, other_run.run_id);
    let source = f.source();
    f.db.record_summary_reads(
        f.community,
        &f.owner(),
        &id,
        a.run_id,
        f.channel,
        std::slice::from_ref(&source),
    )
    .await
    .unwrap();
    let report = f.report(&id, &a, &source);
    assert!(
        f.db.publish_summary_report(f.community, &report)
            .await
            .unwrap()
            .1
    );
    assert!(
        !f.db
            .publish_summary_report(f.community, &report)
            .await
            .unwrap()
            .1
    );
    assert_eq!(
        f.db.summary_cursors(f.community, &f.owner(), &id)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(f
        .db
        .summary_cursors(f.community, &f.owner(), &other)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        f.db.summary_run(f.community, &f.owner(), &id, a.run_id)
            .await
            .unwrap()
            .state,
        "published"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn summary_unobserved_cursor_rolls_back_report_and_cursor() {
    let f = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    let config = f.config(&id).await;
    let (run, _) = f.claim(&id, &config).await;
    let report = f.report(&id, &run, &f.source());
    let error =
        f.db.publish_summary_report(f.community, &report)
            .await
            .unwrap_err();
    assert!(error.to_string().contains("not observed"), "{error}");
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
            .bind(f.community.as_uuid())
            .bind(report.id.as_bytes().as_slice())
            .fetch_one(&f.db.pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
    assert!(f
        .db
        .summary_cursors(f.community, &f.owner(), &id)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        f.db.summary_run(f.community, &f.owner(), &id, run.run_id)
            .await
            .unwrap()
            .state,
        "running"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn summary_private_source_revocation_blocks_commit() {
    let f = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    let config = f.config(&id).await;
    let (run, _) = f.claim(&id, &config).await;
    let source = f.source();
    f.db.record_summary_reads(
        f.community,
        &f.owner(),
        &id,
        run.run_id,
        f.channel,
        std::slice::from_ref(&source),
    )
    .await
    .unwrap();
    sqlx::query("UPDATE channel_members SET removed_at=NOW() WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3")
        .bind(f.community.as_uuid()).bind(f.channel).bind(f.people[1].public_key().to_bytes().as_slice()).execute(&f.db.pool).await.unwrap();
    let error =
        f.db.publish_summary_report(f.community, &f.report(&id, &run, &source))
            .await
            .unwrap_err();
    assert!(
        error.to_string().contains("source access revoked"),
        "{error}"
    );
    assert!(f
        .db
        .summary_cursors(f.community, &f.owner(), &id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn summary_people_include_offline_humans_exclude_agents_and_unknown_profiles() {
    let f = Fixture::new().await;
    let unknown = Keys::generate();
    sqlx::query("INSERT INTO relay_members(community_id,pubkey,role) VALUES($1,$2,'member')")
        .bind(f.community.as_uuid())
        .bind(unknown.public_key().to_hex())
        .execute(&f.db.pool)
        .await
        .unwrap();
    let people = f.db.summary_people(f.community).await.unwrap();
    assert_eq!(
        people,
        f.people.iter().map(|k| k.public_key().to_hex()).collect()
    );
    assert!(!people.contains(&f.reporter.public_key().to_hex()));
    assert!(!people.contains(&f.unrelated.public_key().to_hex()));
    assert!(!people.contains(&unknown.public_key().to_hex()));
    // Legacy retired agents can have no agent metadata or bot memberships.
    // The authoritative archive must still exclude them from human audiences.
    sqlx::query("INSERT INTO archived_identities(community_id,pubkey,consent_path,actor,request_event_id) VALUES($1,$2,'admin',$3,$4)")
        .bind(f.community.as_uuid())
        .bind(f.people[1].public_key().to_hex())
        .bind(f.people[0].public_key().to_hex())
        .bind("00".repeat(32))
        .execute(&f.db.pool).await.unwrap();
    let active_people = f.db.summary_people(f.community).await.unwrap();
    assert!(!active_people.contains(&f.people[1].public_key().to_hex()));
    assert!(active_people.contains(&f.people[0].public_key().to_hex()));
    assert!(active_people.contains(&f.people[2].public_key().to_hex()));
    assert!(f
        .db
        .summary_people(CommunityId::from_uuid(Uuid::new_v4()))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn summary_legacy_migration_preserves_cursor_and_history() {
    let f = Fixture::new().await;
    let source = f.source();
    let old=EventBuilder::new(Kind::Custom(30623),serde_json::json!({"v":1,"reporter_pubkey":f.reporter.public_key().to_hex(),"channels":[],"dms":[f.channel]}).to_string())
        .tags([Tag::parse(["d","summary"]).unwrap()]).custom_created_at(nostr::Timestamp::from(source.created_at.as_secs()-2)).sign_with_keys(&f.people[0]).unwrap();
    f.db.replace_parameterized_event(f.community, &old, "summary", None)
        .await
        .unwrap();
    let watermark=EventBuilder::new(Kind::Custom(30624),serde_json::json!({"v":1,"channel_id":f.channel,"last_event_id":source.id.to_hex(),"last_created_at":source.created_at.as_secs()}).to_string())
        .tags([Tag::parse(["d",&format!("wm:{}",f.channel)]).unwrap()]).sign_with_keys(&f.reporter).unwrap();
    f.db.replace_parameterized_event(f.community, &watermark, &format!("wm:{}", f.channel), None)
        .await
        .unwrap();
    let config = SummaryDefinition {
        v: 2,
        summary_id: "default".into(),
        name: "Existing daily report".into(),
        reporter_pubkey: f.reporter.public_key().to_hex(),
        audience: SummaryAudience::OnlyMe,
        read_delegation: true,
        channels: vec![],
        dms: vec![f.channel],
        exclude: vec![],
        auto_include: true,
        schedule: None,
        timezone: "America/New_York".into(),
        paused: false,
        archived: false,
    };
    let migrated = EventBuilder::new(Kind::Custom(30623), serde_json::to_string(&config).unwrap())
        .tags([Tag::parse(["d", "summary"]).unwrap()])
        .sign_with_keys(&f.people[0])
        .unwrap();
    assert!(
        f.db.replace_summary_definition(f.community, &migrated, "summary")
            .await
            .unwrap()
            .1
    );
    let cursors =
        f.db.summary_cursors(f.community, &f.owner(), "default")
            .await
            .unwrap();
    assert_eq!(cursors.len(), 1);
    assert_eq!(cursors[0].event_id, source.id.to_hex());
    assert!(
        !f.db
            .replace_summary_definition(f.community, &migrated, "summary")
            .await
            .unwrap()
            .1
    );
    assert_eq!(
        f.db.summary_cursors(f.community, &f.owner(), "default")
            .await
            .unwrap(),
        cursors
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn summary_reporter_cannot_expand_frozen_audience() {
    let f = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    let config = f.config(&id).await;
    let (run, _) = f.claim(&id, &config).await;
    let source = f.source();
    f.db.record_summary_reads(
        f.community,
        &f.owner(),
        &id,
        run.run_id,
        f.channel,
        std::slice::from_ref(&source),
    )
    .await
    .unwrap();
    let original = f.report(&id, &run, &source);
    let attack = EventBuilder::new(original.kind, &original.content)
        .tags(original.tags.iter().cloned())
        .tags([Tag::parse(["p", &f.people[2].public_key().to_hex()]).unwrap()])
        .allow_self_tagging()
        .sign_with_keys(&f.reporter)
        .unwrap();
    let error =
        f.db.publish_summary_report(f.community, &attack)
            .await
            .unwrap_err();
    assert!(error.to_string().contains("frozen run audience"), "{error}");
    assert!(f
        .db
        .summary_cursors(f.community, &f.owner(), &id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn summary_expired_run_recovery_does_not_advance_cursors() {
    let f = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    let config = f.config(&id).await;
    let (first, _) = f.claim(&id, &config).await;
    sqlx::query("UPDATE summary_runs SET expires_at=NOW()-INTERVAL '1 second' WHERE community_id=$1 AND run_id=$2")
        .bind(f.community.as_uuid()).bind(first.run_id).execute(&f.db.pool).await.unwrap();
    let (next, created) = f.claim(&id, &config).await;
    assert!(created);
    assert_ne!(first.run_id, next.run_id);
    assert_eq!(
        f.db.summary_run(f.community, &f.owner(), &id, first.run_id)
            .await
            .unwrap()
            .state,
        "failed"
    );
    assert!(f
        .db
        .summary_cursors(f.community, &f.owner(), &id)
        .await
        .unwrap()
        .is_empty());
    assert!(f
        .db
        .summary_run(
            CommunityId::from_uuid(Uuid::new_v4()),
            &f.owner(),
            &id,
            next.run_id
        )
        .await
        .is_err());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn summary_failure_and_approval_preserve_boundaries_and_terminal_receipts() {
    let f = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    let config = f.config(&id).await;
    let (run, _) = f.claim(&id, &config).await;
    f.db.update_summary_run_state(
        f.community,
        &f.owner(),
        &id,
        run.run_id,
        "needs_approval",
        Some("Owner approval needed"),
    )
    .await
    .unwrap();
    assert_eq!(f.claim(&id, &config).await.0.run_id, run.run_id);
    f.db.update_summary_run_state(f.community, &f.owner(), &id, run.run_id, "running", None)
        .await
        .unwrap();
    f.db.update_summary_run_state(
        f.community,
        &f.owner(),
        &id,
        run.run_id,
        "failed",
        Some("Model unavailable"),
    )
    .await
    .unwrap();
    assert!(f
        .db
        .update_summary_run_state(f.community, &f.owner(), &id, run.run_id, "running", None)
        .await
        .is_err());
    assert!(f
        .db
        .summary_cursors(f.community, &f.owner(), &id)
        .await
        .unwrap()
        .is_empty());
    let restarted = crate::Db::from_pool(f.db.pool.clone());
    assert_eq!(
        restarted
            .summary_run(f.community, &f.owner(), &id, run.run_id)
            .await
            .unwrap()
            .state,
        "failed"
    );
    assert_ne!(f.claim(&id, &config).await.0.run_id, run.run_id);
}
