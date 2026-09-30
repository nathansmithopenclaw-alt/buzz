//! Profile reads preserve the full signed snapshot for later durable edits.
use super::{profile_avatar, query_relay_at, relay_http_base_url};
use crate::app_state::AppState;
use nostr::Keys;

// ── Agent profile query ─────────────────────────────────────────────────────

/// Query the relay for an agent's kind:0 profile event.
///
/// Callers pass the active workspace relay resolved by
/// `effective_agent_relay_url`. Returns parsed fields, full metadata, and event
/// timestamp, or `None` when no kind:0 profile is published.
pub async fn query_agent_profile(
    state: &AppState,
    relay_url: &str,
    agent_pubkey: &str,
) -> Result<Option<AgentProfileInfo>, String> {
    let filter = serde_json::json!({
        "authors": [agent_pubkey],
        "kinds": [0],
        "limit": 1
    });

    let events = query_relay_at(state, &relay_http_base_url(relay_url), &[filter]).await?;

    let Some(event) = events.first() else {
        return Ok(None);
    };

    parse_agent_profile(event)
}

/// Read a managed profile using its pinned signer and relay throughout the request.
pub(crate) async fn query_agent_profile_with_keys(
    state: &AppState,
    relay_url: &str,
    keys: &Keys,
    auth_tag: Option<&str>,
) -> Result<Option<AgentProfileInfo>, String> {
    match profile_avatar::query_profile_event(state, relay_url, keys, auth_tag).await? {
        Some(event) => parse_agent_profile(&event),
        None => Ok(None),
    }
}

fn parse_agent_profile(event: &nostr::Event) -> Result<Option<AgentProfileInfo>, String> {
    let Ok(content) = serde_json::from_str::<serde_json::Value>(&event.content) else {
        return Ok(None);
    };

    Ok(Some(AgentProfileInfo {
        display_name: content
            .get("display_name")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        picture: content
            .get("picture")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        about: content
            .get("about")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        metadata: content.as_object().cloned().unwrap_or_default(),
        tags: event.tags.iter().cloned().collect(),
        created_at: event.created_at.as_secs() as i64,
    }))
}

/// Parsed fields from a kind:0 profile event.
#[derive(Debug, Clone)]
pub struct AgentProfileInfo {
    pub display_name: Option<String>,
    pub picture: Option<String>,
    pub about: Option<String>,
    pub metadata: serde_json::Map<String, serde_json::Value>,
    pub tags: Vec<nostr::Tag>,
    pub created_at: i64,
}
