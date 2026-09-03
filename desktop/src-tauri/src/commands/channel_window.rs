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
    let filter = build_channel_window_filter(
        &channel_id,
        limit_rows.unwrap_or(50).min(200),
        cursor.as_ref(),
    );
    let my_pubkey = state
        .keys
        .lock()
        .map_err(|error| error.to_string())?
        .public_key()
        .to_hex();
    let mut approval_filter = serde_json::json!({
        "kinds": [buzz_core_pkg::kind::KIND_WORKFLOW_APPROVAL_REQUESTED],
        "#h": [channel_id],
        "#p": [my_pubkey],
        "limit": limit_rows.unwrap_or(50).min(200),
        "top_level": true,
    });
    if let Some(cursor) = cursor.as_ref() {
        approval_filter["until"] = serde_json::json!(cursor.created_at);
        approval_filter["before_id"] = serde_json::json!(cursor.event_id);
    }
    Ok(query_relay(&state, &[filter, approval_filter])
        .await?
        .iter()
        .filter_map(|event| serde_json::to_value(event).ok())
        .collect())
}
