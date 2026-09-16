//! Explicit bounded automation approvals shared by Desktop and the dispatcher.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Whether an automation approval may produce new effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationMode {
    /// Stop new effects and hold any pending publications.
    Paused,
    /// Evaluate candidates without publishing work or notices.
    Preview,
    /// Permit only the explicitly bounded rollout.
    Active,
}
/// One explicit route; issue ownership remains separate from the executor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationRoute {
    /// Exact issue capability label.
    pub label: String,
    /// Executor public key.
    pub executor: String,
    /// Authorized destination conversation.
    pub channel: Uuid,
}
/// Controls are permissioned shared records, never secret-bearing host configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationControls {
    /// Host service slug whose loop consumes this approval.
    pub service_id: String,
    /// Exact expected heartbeat and execution signer.
    pub service_pubkey: String,
    /// Pause, preview, or explicitly bounded effects.
    pub mode: AutomationMode,
    /// Deadline after which active approvals stop producing effects.
    pub expires_at: Option<u64>,
    /// Maximum distinct actions in this approval, including pending reservations.
    pub max_actions_total: u16,
    /// Maximum distinct new actions during one tick.
    pub max_actions_per_tick: u8,
    /// Only these existing issues may be routed or escalated.
    pub issue_ids: Vec<Uuid>,
    /// Only new pins in these conversations may be captured after approval creation.
    pub capture_channels: Vec<Uuid>,
    /// Explicit label to executor routes.
    pub routing: Vec<AutomationRoute>,
    /// Notify about unrouted triage arrivals; disabled by default in the editor.
    pub notify_triage: bool,
    /// Escalate unresolved unowned work only after this duration; None disables it.
    pub unassigned_after_secs: Option<u64>,
    /// Destination for permitted human notices.
    pub notification_channel: Uuid,
}
impl AutomationControls {
    /// Validate bounds independent of the current clock and host identity.
    pub fn validate(&self) -> bool {
        let key = |value: &str| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        };
        !self.service_id.is_empty()
            && self.service_id.len() <= 64
            && self.service_id.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
            })
            && key(&self.service_pubkey)
            && (1..=50).contains(&self.max_actions_total)
            && (1..=10).contains(&self.max_actions_per_tick)
            && u16::from(self.max_actions_per_tick) <= self.max_actions_total
            && self.issue_ids.len() <= 50
            && self.issue_ids.iter().all(|id| !id.is_nil())
            && self.capture_channels.len() <= 20
            && self.capture_channels.iter().all(|id| !id.is_nil())
            && self.routing.len() <= 20
            && self.routing.iter().all(|route| {
                !route.label.trim().is_empty()
                    && route.label == route.label.trim().to_lowercase()
                    && route.label.len() <= 64
                    && key(&route.executor)
                    && !route.channel.is_nil()
            })
            && !self.notification_channel.is_nil()
            && self
                .unassigned_after_secs
                .is_none_or(|seconds| (3600..=30 * 86400).contains(&seconds))
            && (self.mode != AutomationMode::Active || self.expires_at.is_some())
    }
    /// Active approval cannot run after its deadline, or without a finite scope.
    pub fn allows_effects(&self, now: u64) -> bool {
        self.validate()
            && self.mode == AutomationMode::Active
            && self.expires_at.is_some_and(|deadline| deadline > now)
            && (!self.issue_ids.is_empty() || !self.capture_channels.is_empty())
    }
}
