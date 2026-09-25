//! Version admission for the qualified native Goose connection projection.

/// Absent for hosted agents; selects the session-only connection contract.
pub const SESSION_ONLY_ENV: &str = "BUZZ_ACP_GOOSE_SESSION_ONLY";

/// Native Goose 1.45 session extension configuration, before any prompt.
pub const CONFIGURE_DEVELOPER_METHOD: &str = "_goose/unstable/session/extensions/add";

/// Replace only the developer extension with its text-only native tool set.
/// Session MCP declarations remain untouched, including private launch files.
pub fn text_only_developer_params(session_id: &str) -> serde_json::Value {
    serde_json::json!({
        "sessionId": session_id,
        "extension": {
            "type": "platform", "name": "developer", "bundled": true,
            "available_tools": ["edit", "shell", "tree", "write"]
        }
    })
}

/// Unknown/new CLI schemas must pass isolation controls before admission.
pub fn validate_version(version: Option<(u64, u64, u64)>) -> Result<(), String> {
    if version == Some((1, 45, 0)) {
        return Ok(());
    }
    let actual = version
        .map(|(a, b, c)| format!("{a}.{b}.{c}"))
        .unwrap_or_else(|| "unknown".into());
    Err(format!(
        "Saved-connection Goose requires qualified CLI 1.45.0; installed version is {actual}."
    ))
}
