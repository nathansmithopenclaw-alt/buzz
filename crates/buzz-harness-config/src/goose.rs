//! Version admission for the qualified native Goose connection projection.

/// Absent for hosted agents; selects the session-only connection contract.
pub const SESSION_ONLY_ENV: &str = "BUZZ_ACP_GOOSE_SESSION_ONLY";

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
