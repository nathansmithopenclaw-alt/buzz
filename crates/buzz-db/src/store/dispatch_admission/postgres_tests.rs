use super::*;
use nostr::{EventBuilder, Keys, Kind, Tag};
use sqlx::PgPool;
struct Fixture {
    db: Db,
    community: CommunityId,
    workspace: Uuid,
    author: Keys,
    relay: Keys,
}
impl Fixture {
    async fn new() -> Self {
        let db = Db::from_pool(
            PgPool::connect(&crate::test_support::database_url())
                .await
                .unwrap(),
        );
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let workspace = Uuid::new_v4();
        let author = Keys::generate();
        let relay = Keys::generate();
        sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
            .bind(community.as_uuid())
            .bind(format!("admission-{}.test", workspace))
            .execute(&db.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users(community_id,pubkey) VALUES($1,$2)")
            .bind(community.as_uuid())
            .bind(author.public_key().to_bytes().as_slice())
            .execute(&db.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO channels(community_id,id,name,created_by,channel_type,visibility) VALUES($1,$2,'admission fixture',$3,'stream','private')").bind(community.as_uuid()).bind(workspace).bind(author.public_key().to_bytes().as_slice()).execute(&db.pool).await.unwrap();
        sqlx::query("INSERT INTO channel_members(community_id,channel_id,pubkey,role) VALUES($1,$2,$3,'owner')").bind(community.as_uuid()).bind(workspace).bind(author.public_key().to_bytes().as_slice()).execute(&db.pool).await.unwrap();
        let roster = EventBuilder::new(Kind::Custom(39002), "")
            .tags([
                Tag::parse(["d", &workspace.to_string()]).unwrap(),
                Tag::parse(["p", &author.public_key().to_hex(), "", "owner"]).unwrap(),
            ])
            .sign_with_keys(&relay)
            .unwrap();
        db.insert_event(community, &roster, Some(workspace))
            .await
            .unwrap();
        Self {
            db,
            community,
            workspace,
            author,
            relay,
        }
    }
    fn source(&self, body: &str) -> Event {
        EventBuilder::new(Kind::Custom(40020), body)
            .tags([Tag::parse(["h", &self.workspace.to_string()]).unwrap()])
            .sign_with_keys(&self.author)
            .unwrap()
    }
    fn receipt(
        &self,
        source: &Event,
        role: Option<&str>,
        now: u64,
    ) -> std::result::Result<Event, String> {
        EventBuilder::new(
            Kind::Custom(40027),
            serde_json::json!({"role":role,"observed_at":now}).to_string(),
        )
        .tags([
            Tag::parse(["h", &self.workspace.to_string()]).unwrap(),
            Tag::parse(["e", &source.id.to_hex()]).unwrap(),
        ])
        .sign_with_keys(&self.relay)
        .map_err(|e| e.to_string())
    }
    async fn cleanup(&self) {
        for statement in [
            "DELETE FROM event_mentions WHERE community_id=$1",
            "DELETE FROM thread_metadata WHERE community_id=$1",
            "DELETE FROM events WHERE community_id=$1",
            "DELETE FROM channels WHERE community_id=$1",
            "DELETE FROM users WHERE community_id=$1",
            "DELETE FROM relay_journal_checkpoints WHERE community_id=$1",
            "DELETE FROM relay_event_journal WHERE community_id=$1",
            "DELETE FROM relay_journal_state WHERE community_id=$1",
            "DELETE FROM communities WHERE id=$1",
        ] {
            sqlx::query(statement)
                .bind(self.community.as_uuid())
                .execute(&self.db.pool)
                .await
                .unwrap();
        }
    }
}
#[tokio::test]
#[ignore = "requires Postgres"]
async fn receipt_is_atomic_and_duplicate_retry_does_not_reobserve_role() {
    let f = Fixture::new().await;
    let source = f.source("first");
    let (_, inserted, receipt) =
        f.db.insert_dispatch_with_admission(
            f.community,
            &source,
            f.workspace,
            None,
            None,
            |role, now| f.receipt(&source, role, now),
        )
        .await
        .unwrap();
    assert!(inserted);
    assert_eq!(
        f.db.dispatch_client_history(f.community, f.workspace)
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&receipt.unwrap().event.content).unwrap()["role"],
        "owner"
    );
    sqlx::query("UPDATE channel_members SET role='member' WHERE community_id=$1 AND channel_id=$2")
        .bind(f.community.as_uuid())
        .bind(f.workspace)
        .execute(&f.db.pool)
        .await
        .unwrap();
    let (_, inserted, receipt) = f
        .db
        .insert_dispatch_with_admission(f.community, &source, f.workspace, None, None, |_, _| {
            panic!("retry must not replace original role")
        })
        .await
        .unwrap();
    assert!(!inserted);
    assert!(receipt.is_none());
    let failed = f.source("rolled back");
    assert!(f
        .db
        .insert_dispatch_with_admission(f.community, &failed, f.workspace, None, None, |_, _| Err(
            "simulated signing failure".into()
        ))
        .await
        .is_err());
    assert!(f
        .db
        .get_event_by_id_for_event_write(f.community, failed.id.as_bytes())
        .await
        .unwrap()
        .is_none());
    f.cleanup().await;
}
#[tokio::test]
#[ignore = "requires Postgres"]
async fn membership_mutation_lock_precedes_admission_role_read() {
    let f = Fixture::new().await;
    let source = f.source("serialized");
    let mut tx = f.db.pool.begin().await.unwrap();
    crate::store::channel_members::acquire_channel_membership_lock(
        &mut tx,
        f.community,
        f.workspace,
    )
    .await
    .unwrap();
    let insert = f.db.insert_dispatch_with_admission(
        f.community,
        &source,
        f.workspace,
        None,
        None,
        |role, now| {
            assert_eq!(role, Some("member"));
            f.receipt(&source, role, now)
        },
    );
    tokio::pin!(insert);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(80), &mut insert)
            .await
            .is_err()
    );
    sqlx::query("UPDATE channel_members SET role='member' WHERE community_id=$1 AND channel_id=$2")
        .bind(f.community.as_uuid())
        .bind(f.workspace)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(insert.await.unwrap().1);
    f.cleanup().await;
}
#[tokio::test]
#[ignore = "requires Postgres"]
async fn baseline_preview_changes_with_roles_and_never_writes_until_digest_matches() {
    let f = Fixture::new().await;
    let source = f.source("legacy");
    f.db.insert_event(f.community, &source, Some(f.workspace))
        .await
        .unwrap();
    let preview =
        f.db.baseline_dispatch_admissions(
            f.community,
            f.workspace,
            &f.relay.public_key().to_bytes(),
            None,
            None,
            |_, _, _| panic!("preview must not sign"),
        )
        .await
        .unwrap();
    assert_eq!(preview.count, 1);
    assert!(preview.receipts.is_empty());
    sqlx::query("UPDATE channel_members SET role='member' WHERE community_id=$1 AND channel_id=$2")
        .bind(f.community.as_uuid())
        .bind(f.workspace)
        .execute(&f.db.pool)
        .await
        .unwrap();
    assert!(f
        .db
        .baseline_dispatch_admissions(
            f.community,
            f.workspace,
            &f.relay.public_key().to_bytes(),
            Some(&preview.digest),
            None,
            |_, _, _| panic!("stale preview must not sign")
        )
        .await
        .is_err());
    let preview =
        f.db.baseline_dispatch_admissions(
            f.community,
            f.workspace,
            &f.relay.public_key().to_bytes(),
            None,
            None,
            |_, _, _| panic!(),
        )
        .await
        .unwrap();
    let applied =
        f.db.baseline_dispatch_admissions(
            f.community,
            f.workspace,
            &f.relay.public_key().to_bytes(),
            Some(&preview.digest),
            None,
            |source, role, now| f.receipt(source, role, now),
        )
        .await
        .unwrap();
    assert_eq!(applied.receipts.len(), 1);
    let empty =
        f.db.baseline_dispatch_admissions(
            f.community,
            f.workspace,
            &f.relay.public_key().to_bytes(),
            None,
            None,
            |_, _, _| panic!(),
        )
        .await
        .unwrap();
    assert_eq!(empty.count, 0);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn oversized_projection_is_rejected_before_loading_event_contents() {
    let f = Fixture::new().await;
    let source = f.source(&"x".repeat(16 * 1024 * 1024));
    f.db.insert_event(f.community, &source, Some(f.workspace))
        .await
        .unwrap();
    let error =
        f.db.dispatch_client_history(f.community, f.workspace)
            .await
            .unwrap_err();
    assert!(error.to_string().contains("16 MiB"));
    assert!(f
        .db
        .dispatch_client_history(f.community, Uuid::new_v4())
        .await
        .unwrap()
        .is_empty());
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn task_admission_preserves_thread_metadata_atomically() {
    let f = Fixture::new().await;
    let source = EventBuilder::new(Kind::Custom(9), "Task fixture")
        .tags([
            Tag::parse(["h", &f.workspace.to_string()]).unwrap(),
            Tag::parse(["t", "buzz-task"]).unwrap(),
        ])
        .sign_with_keys(&f.author)
        .unwrap();
    let timestamp =
        chrono::DateTime::from_timestamp(source.created_at.as_secs() as i64, 0).unwrap();
    let meta = super::super::event::ThreadMetadataParams {
        event_id: source.id.as_bytes(),
        event_created_at: timestamp,
        channel_id: f.workspace,
        parent_event_id: None,
        parent_event_created_at: None,
        root_event_id: None,
        root_event_created_at: None,
        depth: 0,
        broadcast: false,
    };
    let (_, inserted, receipt) =
        f.db.insert_dispatch_with_admission(
            f.community,
            &source,
            f.workspace,
            Some(meta),
            None,
            |role, now| f.receipt(&source, role, now),
        )
        .await
        .unwrap();
    assert!(inserted && receipt.is_some());
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM thread_metadata WHERE community_id=$1 AND event_id=$2",
    )
    .bind(f.community.as_uuid())
    .bind(source.id.as_bytes().as_slice())
    .fetch_one(&f.db.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn concurrent_conversation_updates_commit_once_and_exact_retry_is_safe() {
    let f = Fixture::new().await;
    let key = "ab".repeat(32);
    let build = |body| {
        EventBuilder::new(Kind::Custom(9), body)
            .tags([
                Tag::parse(["h", &f.workspace.to_string()]).unwrap(),
                Tag::parse(["t", "dispatch-update"]).unwrap(),
                Tag::parse(["d", &key]).unwrap(),
            ])
            .sign_with_keys(&f.author)
            .unwrap()
    };
    let a = build("first client");
    let b = build("second client");
    let (one, two) = tokio::join!(
        f.db.insert_dispatch_notice(f.community, &a, f.workspace, &key, None),
        f.db.insert_dispatch_notice(f.community, &b, f.workspace, &key, None)
    );
    assert_ne!(one.is_ok(), two.is_ok());
    let accepted = if one.is_ok() { &a } else { &b };
    let (_, inserted) =
        f.db.insert_dispatch_notice(f.community, accepted, f.workspace, &key, None)
            .await
            .unwrap();
    assert!(!inserted);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND kind=9")
            .bind(f.community.as_uuid())
            .fetch_one(&f.db.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn concurrent_captures_share_one_issue_root_without_discarding_prior_history() {
    let f = Fixture::new().await;
    let capture = Uuid::new_v4();
    let build = |body| {
        EventBuilder::new(Kind::Custom(40020), body)
            .tags([
                Tag::parse(["h", &f.workspace.to_string()]).unwrap(),
                Tag::parse(["d", &capture.to_string()]).unwrap(),
            ])
            .sign_with_keys(&f.author)
            .unwrap()
    };
    let a = build("capture A");
    let b = build("capture B");
    let (one, two) = tokio::join!(
        f.db.insert_dispatch_with_admission(
            f.community,
            &a,
            f.workspace,
            None,
            Some(capture),
            |role, now| f.receipt(&a, role, now)
        ),
        f.db.insert_dispatch_with_admission(
            f.community,
            &b,
            f.workspace,
            None,
            Some(capture),
            |role, now| f.receipt(&b, role, now)
        )
    );
    assert_ne!(one.is_ok(), two.is_ok());
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND kind=40020")
            .bind(f.community.as_uuid())
            .fetch_one(&f.db.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    f.cleanup().await;
}
