//! Defaults shared by relay Autopilot authorization and typed clients.

/// Default simultaneous assignments per executor.
pub fn default_at_a_time() -> u32 {
    2
}
/// Default project assignments per UTC day.
pub fn default_per_day() -> u32 {
    10
}
/// Autopilot always requires independent review.
pub fn default_reviewer_required() -> bool {
    true
}

/// Relay-signed kind-40023 outcome for effects that have no issue yet (capture
/// authorization failures) or need a project-level confirmation. Existing
/// issue-bound `tracker::DispatchAction` records retain their original schema.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowOutcome {
    /// Schema version, currently 1.
    pub v: u8,
    /// Literal `workflow_outcome` discriminant.
    #[serde(rename = "type")]
    pub record_type: String,
    /// Literal workflow action name.
    pub action: String,
    /// Durable scoped idempotency key.
    pub request_key: String,
    /// Source message, signed policy, or workflow run identity.
    pub source: String,
    /// Human-readable result, including `skipped: ...` reasons.
    pub result: String,
    /// Whether an administrator must confirm the first-hour assignments.
    #[serde(default)]
    pub confirmation_required: bool,
    /// Exact third assignment action to name in the confirming Autopilot revision.
    #[serde(default)]
    pub confirmation_event: Option<String>,
}
