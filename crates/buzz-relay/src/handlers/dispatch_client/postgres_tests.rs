use super::*;
use buzz_core::CommunityId;
use buzz_sdk::tracker::{build_tracker_issue, IssueRoot, IssueStatus, TRACKER_VERSION};
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};

#[tokio::test]
#[ignore = "requires Postgres"]
async fn command_route_enforces_current_workspace_access_and_never_writes() {
    let state = crate::state::tests::test_state().await;
    let pool = sqlx::PgPool::connect(&state.config.database_url)
        .await
        .unwrap();
    let community = CommunityId::from_uuid(Uuid::new_v4());
    let tenant = TenantContext::resolved(
        community,
        format!("dispatch-client-{}.test", community.as_uuid()),
    );
    let workspace = Uuid::new_v4();
    let issue = Uuid::new_v4();
    let author = Keys::generate();
    let outsider = Keys::generate();
    sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
        .bind(community.as_uuid())
        .bind(tenant.host())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users(community_id,pubkey) VALUES($1,$2)")
        .bind(community.as_uuid())
        .bind(author.public_key().to_bytes().as_slice())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channels(community_id,id,name,created_by,channel_type,visibility) VALUES($1,$2,'read-only fixture',$3,'stream','private')").bind(community.as_uuid()).bind(workspace).bind(author.public_key().to_bytes().as_slice()).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO channel_members(community_id,channel_id,pubkey,role) VALUES($1,$2,$3,'owner')",
    )
    .bind(community.as_uuid())
    .bind(workspace)
    .bind(author.public_key().to_bytes().as_slice())
    .execute(&pool)
    .await
    .unwrap();
    let root = build_tracker_issue(
        workspace,
        &IssueRoot {
            v: TRACKER_VERSION,
            op: "create".into(),
            issue_id: issue,
            identifier: format!("TEST-{}", issue.simple()),
            title: "Command boundary".into(),
            body: "Private scope".into(),
            status: IssueStatus::Todo,
            priority: None,
            assignee: Some(author.public_key().to_hex()),
            project_id: None,
            due: None,
            estimate_minutes: None,
            home_channel_id: None,
            source_event_id: None,
            workflow: "default-v1".into(),
            labels: vec![],
            external_refs: vec![],
            request_id: None,
            import: None,
        },
    )
    .unwrap()
    .sign_with_keys(&author)
    .unwrap();
    state
        .db
        .insert_event(community, &root, Some(workspace))
        .await
        .unwrap();
    let invoke = |who: &Keys, allowed: Option<Vec<Uuid>>, request: Value, tenant: TenantContext| {
        let event = EventBuilder::new(
            Kind::Custom(40028),
            json!({"v":1,"request":request}).to_string(),
        )
        .tags([Tag::parse(["h", &workspace.to_string()]).unwrap()])
        .sign_with_keys(who)
        .unwrap();
        let auth = IngestAuth::Nip42 {
            pubkey: who.public_key(),
            scopes: vec![buzz_auth::Scope::MessagesRead],
            channel_ids: allowed,
            conn_id: Uuid::new_v4(),
            public_scheme: crate::proxy::PublicScheme::Http,
        };
        let state = state.clone();
        async move { super::super::command_executor::handle_command(&tenant, &state, event, auth).await }
    };
    let snapshot = json!({"operation":"snapshot","issue_id":issue});
    let response = invoke(&author, None, snapshot.clone(), tenant.clone())
        .await
        .unwrap();
    let body: Value =
        serde_json::from_str(response.message.strip_prefix("response:").unwrap()).unwrap();
    assert_eq!(body["workspace"], workspace.to_string());
    assert_eq!(body["issues"][0]["title"], "Command boundary");
    assert_eq!(body["capabilities"]["dispatchNotice"], 1);
    let prepare = json!({"operation":"prepare_issue","issue_id":issue,"expected_tip":root.id.to_hex(),"set":{"status":"done"},"reason":"Reviewed","request_id":Uuid::new_v4()});
    let response = invoke(
        &author,
        Some(vec![workspace]),
        prepare.clone(),
        tenant.clone(),
    )
    .await
    .unwrap();
    let body: Value =
        serde_json::from_str(response.message.strip_prefix("response:").unwrap()).unwrap();
    assert_eq!(body["unsigned"]["kind"], 40021);
    assert!(body["unsigned"]["sig"].is_null());
    assert!(invoke(&outsider, None, snapshot.clone(), tenant.clone())
        .await
        .is_err());
    assert!(invoke(
        &author,
        Some(vec![Uuid::new_v4()]),
        snapshot.clone(),
        tenant.clone()
    )
    .await
    .is_err());
    let foreign = TenantContext::resolved(CommunityId::from_uuid(Uuid::new_v4()), "foreign.test");
    assert!(invoke(&author, None, snapshot.clone(), foreign)
        .await
        .is_err());
    sqlx::query("UPDATE channel_members SET role='member' WHERE community_id=$1 AND channel_id=$2")
        .bind(community.as_uuid())
        .bind(workspace)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        invoke(&author, None, prepare, tenant.clone())
            .await
            .is_err(),
        "demoted members cannot close issues"
    );
    assert!(invoke(&author, None, snapshot.clone(), tenant.clone())
        .await
        .is_ok());
    sqlx::query(
        "UPDATE channel_members SET removed_at=NOW() WHERE community_id=$1 AND channel_id=$2",
    )
    .bind(community.as_uuid())
    .bind(workspace)
    .execute(&pool)
    .await
    .unwrap();
    assert!(invoke(&author, None, snapshot, tenant).await.is_err());
    let (events, users): (i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM events WHERE community_id=$1), (SELECT COUNT(*) FROM users WHERE community_id=$1)").bind(community.as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(events, 1, "no command or prepared revision is persisted");
    assert_eq!(users, 1, "read commands do not create outsider users");
    for statement in [
        "DELETE FROM event_mentions WHERE community_id=$1",
        "DELETE FROM events WHERE community_id=$1",
        "DELETE FROM channels WHERE community_id=$1",
        "DELETE FROM users WHERE community_id=$1",
        "DELETE FROM relay_journal_checkpoints WHERE community_id=$1",
        "DELETE FROM relay_event_journal WHERE community_id=$1",
        "DELETE FROM relay_journal_state WHERE community_id=$1",
        "DELETE FROM communities WHERE id=$1",
    ] {
        sqlx::query(statement)
            .bind(community.as_uuid())
            .execute(&pool)
            .await
            .unwrap();
    }
}
