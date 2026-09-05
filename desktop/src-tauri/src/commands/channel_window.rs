use tauri::State;

use crate::{app_state::AppState, models::ChannelPageCursor, relay::query_relay};

const TIMELINE_KINDS: [u32; 11] = [
    9,
    40002,
    40008,
    40099,
    43001,
    43002,
    43003,
    43004,
    43005,
    43006,
    buzz_core_pkg::kind::KIND_HUDDLE_STARTED,
];

fn build_channel_window_filter(
    channel_id: &str,
    cap: u32,
    cursor: Option<&ChannelPageCursor>,
) -> serde_json::Value {
    let mut filter = serde_json::Map::new();
    filter.insert("#h".to_string(), serde_json::json!([channel_id]));
    filter.insert("kinds".to_string(), serde_json::json!(TIMELINE_KINDS));
    filter.insert("limit".to_string(), serde_json::json!(cap));
    filter.insert("top_level".to_string(), serde_json::json!(true));
    filter.insert("include_summaries".to_string(), serde_json::json!(true));
    filter.insert("include_aux".to_string(), serde_json::json!(true));
    if let Some(cursor) = cursor {
        filter.insert("until".to_string(), serde_json::json!(cursor.created_at));
        filter.insert("before_id".to_string(), serde_json::json!(cursor.event_id));
    }
    serde_json::Value::Object(filter)
}

/// Fetch one server-assembled channel window over the existing `/query` bridge.
#[tauri::command]
pub async fn get_channel_window(
    channel_id: String,
    limit_rows: Option<u32>,
    cursor: Option<ChannelPageCursor>,
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    fetch_channel_window(
        &channel_id,
        limit_rows,
        cursor.as_ref(),
        |filters| async move { query_relay(&state, &filters).await },
    )
    .await
}

async fn fetch_channel_window<F, Fut>(
    channel_id: &str,
    limit_rows: Option<u32>,
    cursor: Option<&ChannelPageCursor>,
    query: F,
) -> Result<Vec<serde_json::Value>, String>
where
    F: FnOnce(Vec<serde_json::Value>) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<nostr::Event>, String>>,
{
    let filter = build_channel_window_filter(channel_id, limit_rows.unwrap_or(50).min(200), cursor);
    // One authoritative row set must have one bounds overlay. A second
    // top_level filter has independent exhaustion/cursor state and cannot be
    // appended to this page. Approval history needs a recipient-aware shared
    // window; until then approvals remain in Needs Action and the live stream.
    Ok(query(vec![filter])
        .await?
        .iter()
        .filter_map(|event| serde_json::to_value(event).ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exercise the actual query invocation, not just a filter helper: the
    // regression appended a second filter after constructing the first one.
    #[tokio::test]
    async fn window_query_has_one_row_set_and_preserves_composite_cursor() {
        let cursor = ChannelPageCursor {
            created_at: 1234,
            event_id: "ab".repeat(32),
        };
        for (limit, expected_cap) in [(None, 50), (Some(2), 2), (Some(999), 200)] {
            for page_cursor in [None, Some(&cursor)] {
                let response =
                    fetch_channel_window("channel", limit, page_cursor, |filters| async move {
                        assert_eq!(filters.len(), 1, "a window has one bounds authority");
                        let filter = &filters[0];
                        assert_eq!(filter["#h"], serde_json::json!(["channel"]));
                        assert_eq!(filter["kinds"], serde_json::json!(TIMELINE_KINDS));
                        assert!(!TIMELINE_KINDS
                            .contains(&buzz_core_pkg::kind::KIND_WORKFLOW_APPROVAL_REQUESTED));
                        assert_eq!(filter["limit"], expected_cap);
                        assert_eq!(filter["top_level"], true);
                        assert_eq!(filter["include_aux"], true);
                        assert_eq!(filter["include_summaries"], true);
                        assert!(filter.get("#p").is_none());
                        if let Some(cursor) = page_cursor {
                            assert_eq!(filter["until"], cursor.created_at);
                            assert_eq!(filter["before_id"], cursor.event_id);
                        } else {
                            assert!(filter.get("until").is_none());
                            assert!(filter.get("before_id").is_none());
                        }
                        Ok(Vec::new())
                    })
                    .await
                    .expect("window query succeeds");
                assert!(response.is_empty());
            }
        }
    }

    #[tokio::test]
    async fn window_query_preserves_relay_bounds_and_errors() {
        let bounds = nostr::EventBuilder::new(
            nostr::Kind::Custom(buzz_core_pkg::kind::KIND_WINDOW_BOUNDS as u16),
            r#"{"has_more":false,"next_cursor":null}"#,
        )
        .sign_with_keys(&nostr::Keys::generate())
        .expect("signed bounds");
        let expected = serde_json::to_value(&bounds).expect("bounds JSON");
        let response = fetch_channel_window("channel", None, None, |_| async { Ok(vec![bounds]) })
            .await
            .expect("window response");
        assert_eq!(response, vec![expected]);
        let error = fetch_channel_window("channel", None, None, |_| async {
            Err("relay unavailable".to_owned())
        })
        .await
        .expect_err("relay errors propagate");
        assert_eq!(error, "relay unavailable");
    }
}
