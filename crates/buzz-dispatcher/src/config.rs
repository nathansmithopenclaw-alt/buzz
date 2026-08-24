//! Dispatcher configuration schema.
//!
//! Every identity/location input is config-derived (binding plan amendment 3):
//! routing table, thresholds, monitored channels, decision surface, liveness
//! owner, timers, and the projection/cursor `state_dir`, which must resolve
//! OUTSIDE the repo — never repo-relative, never in-tree.
//!
//! Secrets never live here: the service key comes from `BUZZ_PRIVATE_KEY`
//! (hex or nsec) and the optional NIP-OA tag from `BUZZ_AUTH_TAG`, exactly
//! the seam `buzz-cli` documents.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::DispatchError;

const DEFAULT_POLL_INTERVAL_SECS: u64 = 30;
const DEFAULT_PIN_EMOJI: &str = "\u{1F4CC}"; // 📌
const DEFAULT_STALE_AFTER_SECS: u64 = 300;
const MAX_SERVICE_ID_LEN: usize = 64;
const MAX_POLL_INTERVAL_SECS: u64 = 3600;
const MAX_WINDOW_SECS: u64 = 365 * 24 * 3600;

/// Top-level dispatcher configuration (strict JSON, unknown fields rejected).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatcherConfig {
    /// Relay base URL (`http://` or `https://`). May be omitted when
    /// `BUZZ_RELAY_URL` is set in the environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_url: Option<String>,
    /// Service slug used in heartbeats (`d` tag and `service_id` field).
    pub service_id: String,
    /// Tracker stream UUID this dispatcher projects.
    pub stream_id: Uuid,
    /// Seconds between projection refresh / rule-evaluation ticks.
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    /// Channels whose pin reactions are captured into triage issues.
    #[serde(default)]
    pub monitored_channels: Vec<Uuid>,
    /// Reaction emoji that triggers capture.
    #[serde(default = "default_pin_emoji")]
    pub pin_emoji: String,
    /// Fallback notification channel when an issue has no home channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_channel: Option<Uuid>,
    /// Label auto-routing table.
    #[serde(default)]
    pub routing: Vec<RouteRule>,
    /// Escalation thresholds (stable config IDs per NIP-BT).
    #[serde(default)]
    pub thresholds: Vec<Threshold>,
    /// Workspace tracker roles by pubkey hex (fold authorization). The
    /// service identity is merged in at runtime with at least Member so its
    /// own captures fold valid; grant Maintainer here for terminal/reopen
    /// authority (M13, deploy-time step).
    #[serde(default)]
    pub roles: std::collections::BTreeMap<String, RoleConfig>,
    /// Where P0/deadline escalations surface for owner decision.
    pub decision_surface: DecisionSurface,
    /// Heartbeat liveness configuration for the external watchdog.
    pub liveness: LivenessConfig,
    /// Absolute directory for the projection + cursor state, outside the repo.
    pub state_dir: PathBuf,
}

fn default_poll_interval() -> u64 {
    DEFAULT_POLL_INTERVAL_SECS
}

fn default_pin_emoji() -> String {
    DEFAULT_PIN_EMOJI.to_string()
}

fn default_stale_after() -> u64 {
    DEFAULT_STALE_AFTER_SECS
}

/// One label auto-routing rule: unassigned triage/todo issues carrying
/// `label` are dispatched to `owner`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteRule {
    /// Exact normalized label to match (e.g. `area:payments`).
    pub label: String,
    /// Owner pubkey (64-char hex) receiving the dispatch mention.
    pub owner: String,
    /// Channel for the dispatch message; falls back to
    /// [`DispatcherConfig::default_channel`] when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<Uuid>,
}

/// One escalation threshold with a stable config ID.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Threshold {
    /// Stable threshold ID (e.g. `unassigned_24h`); part of the
    /// deterministic request ID, so it must never be renamed casually.
    pub id: String,
    /// When the threshold applies.
    pub condition: ThresholdCondition,
    /// Qualifying interval in seconds. At-most-once firing per
    /// `(issue, threshold, epoch)` where `epoch = floor(entry_time / window)`.
    pub window_secs: u64,
    /// What happens when the threshold fires.
    pub action: ThresholdAction,
}

/// Workspace tracker role for fold authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleConfig {
    /// May create, edit, relate, and comment.
    Member,
    /// Additionally may make terminal/reopen transitions and renumber issues.
    Maintainer,
}

impl RoleConfig {
    /// The SDK fold role equivalent.
    pub fn to_tracker_role(self) -> buzz_sdk::tracker::TrackerRole {
        match self {
            RoleConfig::Member => buzz_sdk::tracker::TrackerRole::Member,
            RoleConfig::Maintainer => buzz_sdk::tracker::TrackerRole::Maintainer,
        }
    }
}

/// Condition under which a threshold qualifies an issue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ThresholdCondition {
    /// Issue is unassigned (in a non-terminal status).
    Unassigned,
    /// Issue is unassigned and carries the given label.
    UnassignedWithLabel {
        /// Required label (e.g. `priority:p0`).
        label: String,
    },
    /// Issue is assigned and its due value has passed.
    DeadlinePassed,
}

/// Effect when a threshold fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThresholdAction {
    /// (Re-)post a notice in the issue's home channel.
    NotifyHome,
    /// Coalesce into the periodic digest message for non-top-tier tiers.
    Digest,
    /// Surface on the configured decision surface for owner action.
    DecisionSurface,
}

/// Address of the owner decision surface for top-tier escalations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionSurface {
    /// Channel receiving decision-surface escalations.
    pub channel: Uuid,
}

/// Liveness configuration consumed by the external watchdog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LivenessConfig {
    /// Channel where stale-heartbeat notices are posted.
    pub channel: Uuid,
    /// Pubkey mentioned on stale notices; optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Heartbeats older than this are stale.
    #[serde(default = "default_stale_after")]
    pub stale_after_secs: u64,
    /// Signer pubkeys (64-hex) whose kind-40024 heartbeats count toward
    /// liveness (M6). Any channel member can publish a 40024 with the
    /// service slug, so the SIGNER — not the content — is the proof of
    /// life; unlisted signers are ignored. Rotation is a config edit.
    /// Required and must be non-empty.
    pub signer_allowlist: Vec<String>,
}

impl DispatcherConfig {
    /// Load and validate configuration from a JSON file, applying the
    /// `BUZZ_RELAY_URL` environment fallback for `relay_url`.
    pub fn load(path: &Path) -> Result<Self, DispatchError> {
        let raw = std::fs::read_to_string(path).map_err(|error| {
            DispatchError::Config(format!(
                "cannot read config file {}: {error}",
                path.display()
            ))
        })?;
        let mut config: Self = serde_json::from_str(&raw).map_err(|error| {
            DispatchError::Config(format!(
                "invalid config JSON in {}: {error}",
                path.display()
            ))
        })?;
        if config.relay_url.is_none() {
            match std::env::var("BUZZ_RELAY_URL") {
                Ok(value) if !value.trim().is_empty() => config.relay_url = Some(value),
                _ => {
                    return Err(DispatchError::Config(
                        "relay_url missing from config and BUZZ_RELAY_URL is not set".into(),
                    ))
                }
            }
        }
        config.validate()?;
        Ok(config)
    }

    /// Validate all semantic constraints. Called by [`Self::load`].
    pub fn validate(&self) -> Result<(), DispatchError> {
        let relay = self
            .relay_url
            .as_deref()
            .ok_or_else(|| DispatchError::Config("relay_url is required".into()))?;
        if !(relay.starts_with("http://") || relay.starts_with("https://")) {
            return Err(DispatchError::Config(format!(
                "relay_url must start with http:// or https://, got {relay}"
            )));
        }
        validate_slug(&self.service_id, "service_id")?;
        // NIT: the SDK builders require a v4 stream UUID at runtime
        // (validate_uuid_v4); reject anything else at check-config time
        // instead of first-boot.
        if self.stream_id.get_version() != Some(uuid::Version::Random) {
            return Err(DispatchError::Config(format!(
                "stream_id must be a version-4 UUID, got {} (version {:?})",
                self.stream_id,
                self.stream_id.get_version()
            )));
        }
        if !(1..=MAX_POLL_INTERVAL_SECS).contains(&self.poll_interval_secs) {
            return Err(DispatchError::Config(format!(
                "poll_interval_secs must be 1..={MAX_POLL_INTERVAL_SECS}"
            )));
        }
        if self.pin_emoji.trim().is_empty() {
            return Err(DispatchError::Config("pin_emoji must not be empty".into()));
        }

        let mut seen_labels = std::collections::HashSet::new();
        for rule in &self.routing {
            if rule.label.trim().is_empty() {
                return Err(DispatchError::Config(
                    "routing label must not be empty".into(),
                ));
            }
            if !seen_labels.insert(rule.label.clone()) {
                return Err(DispatchError::Config(format!(
                    "duplicate routing label: {}",
                    rule.label
                )));
            }
            validate_hex_pubkey(&rule.owner, &format!("routing owner for {}", rule.label))?;
            if rule.channel.is_none() && self.default_channel.is_none() {
                return Err(DispatchError::Config(format!(
                    "routing rule {} has no channel and no default_channel fallback",
                    rule.label
                )));
            }
        }

        let mut seen_ids = std::collections::HashSet::new();
        for threshold in &self.thresholds {
            validate_slug(&threshold.id, &format!("threshold id {}", threshold.id))?;
            if !seen_ids.insert(threshold.id.clone()) {
                return Err(DispatchError::Config(format!(
                    "duplicate threshold id: {}",
                    threshold.id
                )));
            }
            if !(1..=MAX_WINDOW_SECS).contains(&threshold.window_secs) {
                return Err(DispatchError::Config(format!(
                    "threshold {} window_secs must be 1..={MAX_WINDOW_SECS}",
                    threshold.id
                )));
            }
            if let ThresholdCondition::UnassignedWithLabel { label } = &threshold.condition {
                if label.trim().is_empty() {
                    return Err(DispatchError::Config(format!(
                        "threshold {} requires a non-empty label",
                        threshold.id
                    )));
                }
            }
        }

        if let Some(owner) = &self.liveness.owner {
            validate_hex_pubkey(owner, "liveness owner")?;
        }
        if self.liveness.stale_after_secs < 10 {
            return Err(DispatchError::Config(
                "liveness stale_after_secs must be >= 10".into(),
            ));
        }
        // M6: the watchdog only trusts allowlisted heartbeat signers. The
        // list is required and non-empty so a misconfigured watchdog never
        // silently trusts every channel member.
        if self.liveness.signer_allowlist.is_empty() {
            return Err(DispatchError::Config(
                "liveness signer_allowlist must contain at least one 64-hex pubkey".into(),
            ));
        }
        let mut seen_signers = std::collections::HashSet::new();
        for (index, pubkey) in self.liveness.signer_allowlist.iter().enumerate() {
            validate_hex_pubkey(pubkey, &format!("liveness signer_allowlist[{index}]"))?;
            if !seen_signers.insert(pubkey.clone()) {
                return Err(DispatchError::Config(format!(
                    "duplicate liveness signer_allowlist entry: {pubkey}"
                )));
            }
        }

        for pubkey in self.roles.keys() {
            validate_hex_pubkey(pubkey, &format!("roles key {pubkey}"))?;
        }

        validate_state_dir(&self.state_dir)?;
        Ok(())
    }

    /// Resolved relay URL (always present after [`Self::load`]).
    pub fn relay_url(&self) -> &str {
        self.relay_url.as_deref().unwrap_or_default()
    }

    /// Fold authorization: configured roles plus the service identity, which
    /// always needs at least Member so the dispatcher's own capture roots and
    /// revisions fold valid. A configured Maintainer grant for the service
    /// pubkey is preserved (M13 terminal/reopen authority).
    pub fn authorization(&self, service_pubkey: &str) -> buzz_sdk::tracker::TrackerAuthorization {
        let mut roles = std::collections::HashMap::new();
        for (pubkey, role) in &self.roles {
            roles.insert(pubkey.clone(), role.to_tracker_role());
        }
        roles
            .entry(service_pubkey.to_string())
            .or_insert(buzz_sdk::tracker::TrackerRole::Member);
        buzz_sdk::tracker::TrackerAuthorization::new(roles)
    }
}

/// Validate a stable config slug: 1-64 chars matching `[a-z0-9][a-z0-9_-]*`
/// (leading char lowercase alphanumeric; underscore and hyphen allowed in the
/// rest). Threshold IDs use underscores (e.g. `unassigned_24h`), service ids
/// hyphens (e.g. `triage-dispatcher`).
pub fn validate_slug(value: &str, field: &str) -> Result<(), DispatchError> {
    if value.is_empty() || value.len() > MAX_SERVICE_ID_LEN {
        return Err(DispatchError::Config(format!(
            "{field} must be 1-{MAX_SERVICE_ID_LEN} characters"
        )));
    }
    let mut chars = value.chars();
    let first = chars.next().unwrap_or(' ');
    let valid_first = first.is_ascii_lowercase() || first.is_ascii_digit();
    let valid_rest =
        chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if !valid_first || !valid_rest {
        return Err(DispatchError::Config(format!(
            "{field} must match [a-z0-9][a-z0-9_-]* (got {value})"
        )));
    }
    Ok(())
}

fn validate_hex_pubkey(value: &str, field: &str) -> Result<(), DispatchError> {
    if value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(DispatchError::Config(format!(
            "{field} must be a 64-char hex pubkey"
        )))
    }
}

fn validate_state_dir(path: &Path) -> Result<(), DispatchError> {
    if !path.is_absolute() {
        return Err(DispatchError::Config(format!(
            "state_dir must be absolute (binding: outside the repo), got {}",
            path.display()
        )));
    }
    if inside_git_checkout(path) {
        return Err(DispatchError::Config(format!(
            "state_dir {} resolves inside a git checkout; it must live outside the repo",
            path.display()
        )));
    }
    Ok(())
}

/// True when `path` or any of its ancestors contains a `.git` entry.
fn inside_git_checkout(path: &Path) -> bool {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut current = Some(resolved.as_path());
    while let Some(dir) = current {
        if dir.join(".git").exists() {
            return true;
        }
        current = dir.parent();
    }
    false
}

/// Load the service identity from the environment: `BUZZ_PRIVATE_KEY`
/// (required, hex or nsec) and `BUZZ_AUTH_TAG` (optional NIP-OA JSON).
pub fn identity_from_env() -> Result<(nostr::Keys, Option<String>), DispatchError> {
    let secret = std::env::var("BUZZ_PRIVATE_KEY").map_err(|_| {
        DispatchError::Identity("BUZZ_PRIVATE_KEY is required (hex or nsec)".into())
    })?;
    let keys = nostr::Keys::parse(secret.trim())
        .map_err(|error| DispatchError::Identity(format!("invalid BUZZ_PRIVATE_KEY: {error}")))?;
    let auth_tag = std::env::var("BUZZ_AUTH_TAG")
        .ok()
        .filter(|value| !value.trim().is_empty());
    Ok((keys, auth_tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_json(state_dir: &Path) -> String {
        format!(
            r#"{{
  "relay_url": "http://localhost:3000",
  "service_id": "triage-dispatcher",
  "stream_id": "{}",
  "monitored_channels": ["{}"],
  "default_channel": "{}",
  "routing": [
    {{ "label": "area:payments", "owner": "{}" }}
  ],
  "thresholds": [
    {{ "id": "unassigned_24h", "condition": {{ "kind": "unassigned" }}, "window_secs": 86400, "action": "notify_home" }},
    {{ "id": "p0_unassigned_4h", "condition": {{ "kind": "unassigned_with_label", "label": "priority:p0" }}, "window_secs": 14400, "action": "decision_surface" }}
  ],
  "decision_surface": {{ "channel": "{}" }},
  "liveness": {{ "channel": "{}", "owner": "{}", "signer_allowlist": ["{}"] }},
  "state_dir": "{}"
}}"#,
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            "a".repeat(64),
            Uuid::new_v4(),
            Uuid::new_v4(),
            "b".repeat(64),
            "c".repeat(64),
            state_dir.display()
        )
    }

    fn write_config(dir: &Path, json: &str) -> PathBuf {
        let path = dir.join("dispatcher.json");
        std::fs::write(&path, json).expect("write");
        path
    }

    #[test]
    fn valid_config_loads_with_defaults() {
        let tmp = tempfile::tempdir().expect("tmp");
        let path = write_config(tmp.path(), &valid_json(tmp.path().join("state").as_path()));
        let config = DispatcherConfig::load(&path).expect("load");
        assert_eq!(config.poll_interval_secs, DEFAULT_POLL_INTERVAL_SECS);
        assert_eq!(config.pin_emoji, DEFAULT_PIN_EMOJI);
        assert_eq!(config.liveness.stale_after_secs, DEFAULT_STALE_AFTER_SECS);
        assert_eq!(config.thresholds.len(), 2);
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let tmp = tempfile::tempdir().expect("tmp");
        let json = valid_json(tmp.path().join("state").as_path()).replace(
            "\"state_dir\"",
            "\"fleet_pubkey\": \"deadbeef\", \"state_dir\"",
        );
        let path = write_config(tmp.path(), &json);
        let error = DispatcherConfig::load(&path).unwrap_err();
        assert!(matches!(error, DispatchError::Config(_)));
    }

    #[test]
    fn relative_state_dir_is_rejected() {
        let tmp = tempfile::tempdir().expect("tmp");
        let json = valid_json(Path::new("./state"));
        let path = write_config(tmp.path(), &json);
        let error = DispatcherConfig::load(&path).unwrap_err();
        assert!(error.to_string().contains("absolute"));
    }

    #[test]
    fn state_dir_inside_git_checkout_is_rejected() {
        let tmp = tempfile::tempdir().expect("tmp");
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).expect("mkdir .git");
        let state = repo.join("dispatcher-state");
        std::fs::create_dir_all(&state).expect("mkdir state");
        let path = write_config(tmp.path(), &valid_json(&state));
        let error = DispatcherConfig::load(&path).unwrap_err();
        assert!(error.to_string().contains("git checkout"));
    }

    #[test]
    fn duplicate_threshold_ids_are_rejected() {
        let tmp = tempfile::tempdir().expect("tmp");
        let json = valid_json(tmp.path().join("state").as_path())
            .replace("\"id\": \"p0_unassigned_4h\"", "\"id\": \"unassigned_24h\"");
        let path = write_config(tmp.path(), &json);
        let error = DispatcherConfig::load(&path).unwrap_err();
        assert!(error.to_string().contains("duplicate threshold id"));
    }

    #[test]
    fn routing_without_any_channel_is_rejected() {
        let tmp = tempfile::tempdir().expect("tmp");
        let json = valid_json(tmp.path().join("state").as_path())
            .replace("\"default_channel\": ", "\"_unused_channel\": ");
        // Strip the default_channel line entirely.
        let json = json
            .lines()
            .filter(|line| !line.contains("_unused_channel"))
            .collect::<Vec<_>>()
            .join("\n");
        let path = write_config(tmp.path(), &json);
        let error = DispatcherConfig::load(&path).unwrap_err();
        assert!(error.to_string().contains("default_channel fallback"));
    }

    #[test]
    fn relay_url_falls_back_to_env() {
        // Process env mutation is race-prone under the multithreaded test
        // runner; serialize every env-touching test on this lock.
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let tmp = tempfile::tempdir().expect("tmp");
        let json = valid_json(tmp.path().join("state").as_path())
            .lines()
            .filter(|line| !line.contains("relay_url"))
            .collect::<Vec<_>>()
            .join("\n");
        let path = write_config(tmp.path(), &json);
        std::env::set_var("BUZZ_RELAY_URL", "http://env-relay:3000");
        let result = DispatcherConfig::load(&path);
        std::env::remove_var("BUZZ_RELAY_URL");
        drop(guard);
        let config = result.expect("load");
        assert_eq!(config.relay_url(), "http://env-relay:3000");
    }

    #[test]
    fn slug_validation_rules() {
        assert!(validate_slug("triage-dispatcher", "service_id").is_ok());
        assert!(validate_slug("unassigned_24h", "threshold id").is_ok());
        assert!(validate_slug("", "service_id").is_err());
        assert!(validate_slug("Triage", "service_id").is_err());
        assert!(validate_slug("-lead", "service_id").is_err());
        assert!(validate_slug(&"a".repeat(65), "service_id").is_err());
    }

    #[test]
    fn bad_pubkey_is_rejected() {
        let tmp = tempfile::tempdir().expect("tmp");
        let json = valid_json(tmp.path().join("state").as_path()).replace(&"a".repeat(64), "zz");
        let path = write_config(tmp.path(), &json);
        let error = DispatcherConfig::load(&path).unwrap_err();
        assert!(error.to_string().contains("64-char hex"));
    }
}
