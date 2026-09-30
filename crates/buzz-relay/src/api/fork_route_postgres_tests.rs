use super::postgres_tests::{
    make_nip98_headers, nip_fi_enforce_test_state_with_verifier, oneshot_request_full,
    same_key_nip98_and_assertion_headers, signed_assertion_for_pubkey,
};
use axum::http::StatusCode;
use nostr::Keys;
use serde_json::Value;
use std::sync::Arc;

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn nip_fi_enforce_fork_routes_reject_impersonation_and_accept_proven_owner() {
    use buzz_core::channel::{ChannelType, ChannelVisibility, MemberRole};

    let base = nip_fi_enforce_test_state_with_verifier()
        .await
        .expect("Postgres and Redis test services");
    let mut state = (*base).clone();
    // This is the configuration that previously left the dev fallback open.
    Arc::make_mut(&mut state.config).require_auth_token = false;
    Arc::make_mut(&mut state.config).memory.enabled = true;
    let state = Arc::new(state);
    let owner = Keys::generate();
    let attacker = Keys::generate();
    let host = format!("fork-admission-{}.local", uuid::Uuid::new_v4().simple());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .expect("community")
        .id;
    let owner_bytes = owner.public_key().to_bytes();
    state
        .db
        .ensure_user(community, &owner_bytes)
        .await
        .expect("owner");
    let channel = state
        .db
        .create_channel(
            community,
            "workflow",
            ChannelType::Stream,
            ChannelVisibility::Private,
            None,
            &owner_bytes,
            None,
        )
        .await
        .expect("channel");
    state
        .db
        .add_member(
            community,
            channel.id,
            &owner_bytes,
            MemberRole::Owner,
            Some(&owner_bytes),
        )
        .await
        .expect("membership");
    let workflow_id = uuid::Uuid::new_v4();
    state
        .db
        .upsert_workflow(
            community,
            workflow_id,
            Some(channel.id),
            &owner_bytes,
            "admission",
            r#"{"trigger":{"on":"manual"},"steps":[]}"#,
            &[0; 32],
        )
        .await
        .expect("workflow");
    let runs_path = format!("/api/workflows/{workflow_id}/runs");
    let query_body = br#"{"context":{"kind":"workspace_public"},"mode":"history"}"#;
    let status_body = br#"{"context":{"kind":"workspace_public"}}"#;
    for (method, path, body) in [
        ("POST", "/memory/v1/query", query_body.as_slice()),
        ("POST", "/memory/v1/status", status_body.as_slice()),
        ("GET", runs_path.as_str(), b"".as_slice()),
    ] {
        let url = format!("https://{host}{path}");
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-pubkey",
            owner.public_key().to_hex().parse().expect("pubkey header"),
        );
        headers.insert(
            buzz_auth::CLIENT_ATTACHED_HEADER,
            format!(
                "Bearer {}",
                signed_assertion_for_pubkey(&attacker.public_key().to_hex())
            )
            .parse()
            .expect("assertion header"),
        );
        let (status, _, response) =
            oneshot_request_full(state.clone(), method, path, &host, headers, body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}: {response:?}");

        // A valid NIP-98 proof must also be paired to the assertion's key.
        let mut headers = make_nip98_headers(&owner, &url, method, body);
        headers.insert(
            buzz_auth::CLIENT_ATTACHED_HEADER,
            format!(
                "Bearer {}",
                signed_assertion_for_pubkey(&attacker.public_key().to_hex())
            )
            .parse()
            .expect("assertion header"),
        );
        let (status, _, response) =
            oneshot_request_full(state.clone(), method, path, &host, headers, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {response:?}");
        assert_eq!(response.as_ref(), b"authorization denied\n", "{path}");

        let headers = same_key_nip98_and_assertion_headers(&owner, &url, method, body);
        let (status, _, response) =
            oneshot_request_full(state.clone(), method, path, &host, headers, body).await;
        assert_eq!(status, StatusCode::OK, "{path}: {response:?}");
        let value: Value = serde_json::from_slice(&response).expect("response JSON");
        if method == "GET" {
            assert_eq!(value, serde_json::json!([]));
        } else {
            assert!(
                value.get("receipt").is_some(),
                "provider was reached: {value}"
            );
        }
    }
}
