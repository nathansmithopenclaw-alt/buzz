use super::postgres_tests::bridge_handler_test_state;
use buzz_core::TenantContext;
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::Value;
fn fresh_tenant(host: &str) -> TenantContext {
    TenantContext::resolved(
        buzz_core::CommunityId::from_uuid(uuid::Uuid::new_v4()),
        host,
    )
}
async fn read(
    state: &std::sync::Arc<crate::state::AppState>,
    tenant: &TenantContext,
    reporter: &str,
    filters: &[Value],
) -> Result<axum::Json<Value>, (axum::http::StatusCode, axum::Json<Value>)> {
    super::query_events_authed(
        state,
        tenant,
        &axum::http::HeaderMap::new(),
        &serde_json::to_vec(filters).expect("filters"),
        nostr::PublicKey::from_hex(reporter).expect("reporter"),
        [42; 32],
        None,
    )
    .await
}
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn summary_read_grants_are_scoped_revocable_and_never_membership() {
    use buzz_core::channel::{ChannelType, ChannelVisibility, MemberRole};
    let state = bridge_handler_test_state()
        .await
        .expect("Summary integration test requires Postgres and Redis");
    let owner = Keys::generate();
    let reporter = Keys::generate();
    let stranger = Keys::generate();
    let host = format!("summary-{}.local", uuid::Uuid::new_v4().simple());
    let community = match state
        .db
        .create_community_with_owner(&host, &owner.public_key().to_hex())
        .await
        .expect("create community")
    {
        buzz_db::CreateCommunityWithOwnerResult::Created(record) => record.id,
        other => panic!("unexpected community result: {other:?}"),
    };
    let tenant = TenantContext::resolved(community, &host);
    let owner_bytes = owner.public_key().to_bytes();
    let room = state
        .db
        .create_channel(
            tenant.community(),
            "summary-dm",
            ChannelType::Dm,
            ChannelVisibility::Private,
            None,
            &owner_bytes,
            None,
        )
        .await
        .expect("create DM");
    state
        .db
        .add_member(
            tenant.community(),
            room.id,
            &owner_bytes,
            MemberRole::Owner,
            Some(&owner_bytes),
        )
        .await
        .expect("owner membership");
    let message = EventBuilder::new(Kind::Custom(9), "private transcript")
        .tags([Tag::parse(["h", &room.id.to_string()]).expect("h tag")])
        .sign_with_keys(&owner)
        .expect("message");
    state
        .db
        .insert_event(tenant.community(), &message, Some(room.id))
        .await
        .expect("store message");
    let mut config = serde_json::json!({"v":1,"read_delegation":true,"reporter_pubkey":reporter.public_key().to_hex(),"dms":[room.id.to_string()],"exclude":[]});
    let grant = EventBuilder::new(Kind::Custom(30623), config.to_string())
        .tags([Tag::parse(["d", "summary"]).expect("d tag")])
        .custom_created_at(nostr::Timestamp::from(
            nostr::Timestamp::now().as_secs() - 2,
        ))
        .sign_with_keys(&owner)
        .expect("grant");
    state
        .db
        .insert_event(tenant.community(), &grant, None)
        .await
        .expect("store grant");
    let filter = serde_json::json!({"summary_owner":owner.public_key().to_hex(),"#h":[room.id.to_string()],"limit":100});
    let response = read(
        &state,
        &tenant,
        &reporter.public_key().to_hex(),
        std::slice::from_ref(&filter),
    )
    .await
    .expect("delegated read");
    assert_eq!(response.0["events"][0]["content"], "private transcript");
    assert_eq!(response.0["events"].as_array().expect("events").len(), 1);
    assert!(!state
        .db
        .is_member(
            tenant.community(),
            room.id,
            &reporter.public_key().to_bytes()
        )
        .await
        .expect("membership unchanged"));
    assert!(read(
        &state,
        &tenant,
        &stranger.public_key().to_hex(),
        std::slice::from_ref(&filter)
    )
    .await
    .is_err());
    let other_tenant = fresh_tenant("other-summary-test.local");
    assert!(read(
        &state,
        &other_tenant,
        &reporter.public_key().to_hex(),
        std::slice::from_ref(&filter)
    )
    .await
    .is_err());
    let attempted_write = EventBuilder::new(Kind::Custom(9), "not permitted")
        .tags([Tag::parse(["h", &room.id.to_string()]).expect("h tag")])
        .sign_with_keys(&reporter)
        .expect("write event");
    let denied_write = crate::handlers::ingest::ingest_event(
        &state,
        &tenant,
        attempted_write,
        crate::handlers::ingest::IngestAuth::Nip42 {
            pubkey: reporter.public_key(),
            scopes: vec![],
            channel_ids: None,
            conn_id: uuid::Uuid::new_v4(),
            public_scheme: crate::proxy::PublicScheme::Https,
        },
    )
    .await;
    assert!(
        denied_write.is_err(),
        "delegated reader must not post to private DM"
    );
    let mut broad = filter.clone();
    broad["kinds"] = serde_json::json!([0]);
    assert!(
        read(&state, &tenant, &reporter.public_key().to_hex(), &[broad])
            .await
            .is_err()
    );
    assert!(read(
        &state,
        &tenant,
        &reporter.public_key().to_hex(),
        &[filter.clone(), filter.clone()]
    )
    .await
    .is_err());
    config["read_delegation"] = Value::Bool(false);
    let revoked = EventBuilder::new(Kind::Custom(30623), config.to_string())
        .tags([Tag::parse(["d", "summary"]).expect("d tag")])
        .custom_created_at(nostr::Timestamp::from(
            nostr::Timestamp::now().as_secs() - 1,
        ))
        .sign_with_keys(&owner)
        .expect("revoke");
    state
        .db
        .insert_event(tenant.community(), &revoked, None)
        .await
        .expect("store revoke");
    assert!(read(
        &state,
        &tenant,
        &reporter.public_key().to_hex(),
        std::slice::from_ref(&filter)
    )
    .await
    .is_err());
}
