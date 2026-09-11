use super::*;
use std::sync::atomic::Ordering;

/// A narrow authenticated read can start before the renderer socket finishes
/// restoring its other subscriptions. It uses the ordinary HTTP admission gate.
async fn read_membership(
    state: &AppState,
    workspace_id: &str,
    managed_signer: &str,
) -> Result<Value, String> {
    let generation = state.workspace_apply_generation.load(Ordering::SeqCst);
    let key = active_cache_key(state, workspace_id, managed_signer)?;
    let keys = state
        .keys
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    if keys.public_key().to_hex() != key.managed_signer {
        return Err("Dispatch membership identity changed".into());
    }
    let events = crate::relay::query_relay_at_with_keys(
        state,
        &crate::relay::relay_http_base_url(&key.relay_url),
        &[json!({"kinds": [39002], "#d": [key.workspace_id], "limit": 1})],
        &keys,
        None,
    )
    .await?;
    if generation != state.workspace_apply_generation.load(Ordering::SeqCst)
        || active_cache_key(state, workspace_id, managed_signer)? != key
        || state
            .keys
            .lock()
            .map_err(|error| error.to_string())?
            .public_key()
            != keys.public_key()
    {
        return Err("Dispatch membership scope changed during read".into());
    }
    let event = events
        .into_iter()
        .filter(|event| {
            event.kind.as_u16() == 39002
                && event.tags.iter().any(|tag| {
                    let values = tag.as_slice();
                    values.first().is_some_and(|value| value == "d")
                        && values.get(1) == Some(&key.workspace_id)
                })
        })
        .max_by_key(|event| (event.created_at, event.id))
        .ok_or("Tracker workspace membership snapshot is unavailable.")?;
    event.verify().map_err(|error| error.to_string())?;
    if authorization_from_membership(&event)
        .role(&key.managed_signer)
        .is_none()
    {
        return Err("Current membership does not allow reading this Dispatch workspace".into());
    }
    serde_json::to_value(event).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn read_tracker_membership(
    workspace_id: String,
    managed_signer: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    read_membership(&state, &workspace_id, &managed_signer).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::HeaderMap, routing::post, Json, Router};
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn native_membership_read_checks_identity_revocation_and_inflight_scope() {
        let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
        crate::relay_admission::reset_rate_limit_gate();
        let relay = Keys::generate();
        let viewer = Keys::generate();
        let other = Keys::generate();
        let workspace = Uuid::new_v4().to_string();
        let snapshot = |member: &Keys| {
            EventBuilder::new(Kind::Custom(39002), "")
                .tags([
                    Tag::parse(["d", &workspace]).unwrap(),
                    Tag::parse(["p", &member.public_key().to_hex(), "", "owner"]).unwrap(),
                ])
                .sign_with_keys(&relay)
                .unwrap()
        };
        let response = Arc::new(Mutex::new(snapshot(&viewer)));
        let queries = Arc::new(Mutex::new(Vec::new()));
        let hold = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let handler_response = response.clone();
        let handler_queries = queries.clone();
        let handler_hold = hold.clone();
        let handler_entered = entered.clone();
        let handler_release = release.clone();
        let router = Router::new().route(
            "/query",
            post(move |headers: HeaderMap, Json(filters): Json<Vec<Value>>| {
                let response = handler_response.clone();
                let queries = handler_queries.clone();
                let hold = handler_hold.clone();
                let entered = handler_entered.clone();
                let release = handler_release.clone();
                async move {
                    assert!(headers
                        .get("authorization")
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .starts_with("Nostr "));
                    queries.lock().unwrap().extend(filters);
                    if hold.load(Ordering::SeqCst) {
                        entered.notify_one();
                        release.notified().await;
                    }
                    Json(vec![response.lock().unwrap().clone()])
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let state = crate::app_state::build_app_state();
        *state.keys.lock().unwrap() = viewer.clone();
        *state.relay_url_override.lock().unwrap() = Some(format!("ws://{address}"));
        let signer = viewer.public_key().to_hex();
        let accepted = read_membership(&state, &workspace, &signer).await.unwrap();
        assert_eq!(accepted["id"], response.lock().unwrap().id.to_hex());
        assert_eq!(
            queries.lock().unwrap().as_slice(),
            &[json!({"kinds": [39002], "#d": [workspace], "limit": 1})]
        );
        assert!(
            read_membership(&state, &workspace, &other.public_key().to_hex())
                .await
                .unwrap_err()
                .contains("identity changed")
        );
        assert_eq!(
            queries.lock().unwrap().len(),
            1,
            "wrong signer must fail before any HTTP request"
        );
        *response.lock().unwrap() = snapshot(&other);
        assert!(read_membership(&state, &workspace, &signer)
            .await
            .unwrap_err()
            .contains("Current membership"));
        *response.lock().unwrap() = snapshot(&viewer);
        assert!(
            read_membership(&state, &Uuid::new_v4().to_string(), &signer)
                .await
                .unwrap_err()
                .contains("unavailable")
        );
        hold.store(true, Ordering::SeqCst);
        let (result, ()) = tokio::join!(read_membership(&state, &workspace, &signer), async {
            entered.notified().await;
            state
                .workspace_apply_generation
                .fetch_add(1, Ordering::SeqCst);
            release.notify_one();
        });
        assert!(result.unwrap_err().contains("scope changed"));
        server.abort();
    }
}
