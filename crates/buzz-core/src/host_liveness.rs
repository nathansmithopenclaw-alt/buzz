//! NIP-AH agent-host declaration and liveness payloads.
//!
//! Both payloads are public allowlist projections. Deserialization rejects
//! unknown fields so paths, environment values, task data, channel data, and
//! other private runtime state cannot hitchhike in a valid event.

use std::collections::HashSet;

use nostr::PublicKey;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Maximum UTF-8 bytes in a host-declaration content body.
pub const HOST_DECLARATION_MAX_CONTENT_BYTES: usize = 4 * 1024;
/// Maximum UTF-8 bytes in a host-status content body.
pub const HOST_STATUS_MAX_CONTENT_BYTES: usize = 64 * 1024;
/// Maximum Unicode scalar values in a human-readable host name.
pub const HOST_NAME_MAX_CHARS: usize = 128;
/// Maximum agent lifecycle entries in one host status.
pub const HOST_STATUS_MAX_AGENTS: usize = 256;
/// Maximum allowed difference between heartbeat `ts` and event `created_at`.
pub const HOST_STATUS_TIMESTAMP_SKEW_SECS: u64 = 15 * 60;

/// Host-liveness payload or coordinate validation error.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HostLivenessError {
    /// The NIP-33 host coordinate is not a canonical UUID.
    #[error("host `d` tag must be a canonical lowercase hyphenated UUID")]
    InvalidHostId,
    /// The JSON content does not satisfy the public allowlist schema.
    #[error("{0}")]
    InvalidContent(String),
}

/// Supported host operating-system families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostPlatform {
    /// Apple macOS.
    Macos,
    /// Linux.
    Linux,
    /// Microsoft Windows.
    Windows,
}

/// Public host declaration stored in kind:30180 content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostDeclaration {
    /// Schema version. Version 1 is the only currently valid value.
    pub v: u8,
    /// Human-readable host name.
    pub name: String,
    /// Host operating-system family.
    pub platform: HostPlatform,
    /// Unix timestamp at which this host identity was first created.
    pub created_at: u64,
}

/// Lifecycle state for an agent hosted by one device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostedAgentState {
    /// The local agent process is running.
    Running,
    /// The local agent process is stopped.
    Stopped,
    /// The host currently reports an agent lifecycle error.
    Error,
}

/// One lifecycle-only agent entry in a kind:30181 status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostedAgentStatus {
    /// Canonical 64-character lowercase hex agent public key.
    pub pubkey: String,
    /// Current local lifecycle state.
    pub state: HostedAgentState,
    /// Unix timestamp at which the agent entered this state.
    pub since: u64,
}

/// Public host heartbeat stored in kind:30181 content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostStatus {
    /// Schema version. Version 1 is the only currently valid value.
    pub v: u8,
    /// Positive, persistent, monotonically increasing sequence per host.
    pub seq: u64,
    /// Unix timestamp at which the host sampled this status.
    pub ts: u64,
    /// Lifecycle-only state for agents assigned to this host.
    pub agents: Vec<HostedAgentStatus>,
}

/// Validate and parse a canonical NIP-AH host UUID.
pub fn parse_host_id(value: &str) -> Result<Uuid, HostLivenessError> {
    let parsed = Uuid::parse_str(value).map_err(|_| HostLivenessError::InvalidHostId)?;
    if parsed.to_string() != value {
        return Err(HostLivenessError::InvalidHostId);
    }
    Ok(parsed)
}

/// Parse and validate a kind:30180 content body.
///
/// `relay_now` is Unix seconds. A host identity may be old, but its declared
/// creation time cannot be zero or more than the relay's normal 15-minute
/// future-skew allowance ahead of current time.
pub fn parse_host_declaration(
    content: &str,
    relay_now: u64,
) -> Result<HostDeclaration, HostLivenessError> {
    if content.len() > HOST_DECLARATION_MAX_CONTENT_BYTES {
        return Err(HostLivenessError::InvalidContent(format!(
            "host declaration content exceeds {HOST_DECLARATION_MAX_CONTENT_BYTES} bytes"
        )));
    }
    let declaration: HostDeclaration = serde_json::from_str(content).map_err(|error| {
        HostLivenessError::InvalidContent(format!("invalid host declaration JSON: {error}"))
    })?;
    if declaration.v != 1 {
        return Err(HostLivenessError::InvalidContent(
            "host declaration `v` must equal 1".into(),
        ));
    }
    if declaration.name.is_empty() || declaration.name.trim() != declaration.name {
        return Err(HostLivenessError::InvalidContent(
            "host declaration `name` must be non-empty and trimmed".into(),
        ));
    }
    let name_chars = declaration.name.chars().count();
    if name_chars > HOST_NAME_MAX_CHARS {
        return Err(HostLivenessError::InvalidContent(format!(
            "host declaration `name` exceeds {HOST_NAME_MAX_CHARS} characters"
        )));
    }
    if declaration.name.chars().any(char::is_control) {
        return Err(HostLivenessError::InvalidContent(
            "host declaration `name` must not contain control characters".into(),
        ));
    }
    if declaration.created_at == 0
        || declaration.created_at > relay_now.saturating_add(HOST_STATUS_TIMESTAMP_SKEW_SECS)
    {
        return Err(HostLivenessError::InvalidContent(
            "host declaration `created_at` is not a sane Unix timestamp".into(),
        ));
    }
    Ok(declaration)
}

/// Parse and validate a kind:30181 content body.
///
/// The relay cannot enforce monotonicity against an event that lost the
/// NIP-33 replacement race, but it rejects zero sequence numbers and requires
/// `ts` to remain close to the signed event timestamp. Clients compare the
/// winning head's sequence with prior observations as an additional signal.
pub fn parse_host_status(
    content: &str,
    event_created_at: u64,
) -> Result<HostStatus, HostLivenessError> {
    if content.len() > HOST_STATUS_MAX_CONTENT_BYTES {
        return Err(HostLivenessError::InvalidContent(format!(
            "host status content exceeds {HOST_STATUS_MAX_CONTENT_BYTES} bytes"
        )));
    }
    let status: HostStatus = serde_json::from_str(content).map_err(|error| {
        HostLivenessError::InvalidContent(format!("invalid host status JSON: {error}"))
    })?;
    if status.v != 1 {
        return Err(HostLivenessError::InvalidContent(
            "host status `v` must equal 1".into(),
        ));
    }
    if status.seq == 0 {
        return Err(HostLivenessError::InvalidContent(
            "host status `seq` must be positive".into(),
        ));
    }
    if status.ts == 0 || status.ts.abs_diff(event_created_at) > HOST_STATUS_TIMESTAMP_SKEW_SECS {
        return Err(HostLivenessError::InvalidContent(format!(
            "host status `ts` must be positive and within {HOST_STATUS_TIMESTAMP_SKEW_SECS} seconds of event `created_at`"
        )));
    }
    if status.agents.len() > HOST_STATUS_MAX_AGENTS {
        return Err(HostLivenessError::InvalidContent(format!(
            "host status exceeds {HOST_STATUS_MAX_AGENTS} agents"
        )));
    }

    let mut seen_pubkeys = HashSet::with_capacity(status.agents.len());
    for agent in &status.agents {
        if agent.pubkey.len() != 64
            || !agent
                .pubkey
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || PublicKey::from_hex(&agent.pubkey).is_err()
        {
            return Err(HostLivenessError::InvalidContent(
                "host status agent `pubkey` must be a curve-valid 64-character lowercase hex key"
                    .into(),
            ));
        }
        if !seen_pubkeys.insert(agent.pubkey.as_str()) {
            return Err(HostLivenessError::InvalidContent(format!(
                "host status contains duplicate agent pubkey {}",
                agent.pubkey
            )));
        }
        if agent.since == 0 || agent.since > status.ts {
            return Err(HostLivenessError::InvalidContent(
                "host status agent `since` must be positive and no later than `ts`".into(),
            ));
        }
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_786_910_000;
    const PUBKEY: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

    #[test]
    fn host_id_requires_canonical_uuid() {
        assert!(parse_host_id("018f42c2-9bc2-7dc1-a41e-2f77769f0142").is_ok());
        assert!(parse_host_id("018F42C2-9BC2-7DC1-A41E-2F77769F0142").is_err());
        assert!(parse_host_id("018f42c29bc27dc1a41e2f77769f0142").is_err());
        assert!(parse_host_id("").is_err());
    }

    #[test]
    fn declaration_accepts_only_the_public_allowlist() {
        let valid =
            format!(r#"{{"v":1,"name":"Build host","platform":"macos","created_at":{NOW}}}"#);
        assert_eq!(parse_host_declaration(&valid, NOW).unwrap().v, 1);

        let hitchhiker = format!(
            r#"{{"v":1,"name":"Build host","platform":"macos","created_at":{NOW},"path":"/tmp/private"}}"#
        );
        assert!(parse_host_declaration(&hitchhiker, NOW).is_err());
    }

    #[test]
    fn declaration_rejects_bad_names_versions_and_timestamps() {
        for json in [
            format!(r#"{{"v":2,"name":"Host","platform":"linux","created_at":{NOW}}}"#),
            format!(r#"{{"v":1,"name":" Host","platform":"linux","created_at":{NOW}}}"#),
            r#"{"v":1,"name":"Host","platform":"linux","created_at":0}"#.into(),
            format!(
                r#"{{"v":1,"name":"Host","platform":"linux","created_at":{}}}"#,
                NOW + HOST_STATUS_TIMESTAMP_SKEW_SECS + 1
            ),
        ] {
            assert!(
                parse_host_declaration(&json, NOW).is_err(),
                "accepted {json}"
            );
        }
    }

    #[test]
    fn status_accepts_lifecycle_only_payload() {
        let json = format!(
            r#"{{"v":1,"seq":7,"ts":{NOW},"agents":[{{"pubkey":"{PUBKEY}","state":"running","since":{}}}]}}"#,
            NOW - 5
        );
        let parsed = parse_host_status(&json, NOW).unwrap();
        assert_eq!(parsed.seq, 7);
        assert_eq!(parsed.agents[0].state, HostedAgentState::Running);
    }

    #[test]
    fn status_rejects_hitchhikers_duplicates_and_bad_time() {
        let hitchhiker = format!(
            r#"{{"v":1,"seq":1,"ts":{NOW},"agents":[{{"pubkey":"{PUBKEY}","state":"running","since":{NOW},"task":"secret"}}]}}"#
        );
        assert!(parse_host_status(&hitchhiker, NOW).is_err());

        let duplicate = format!(
            r#"{{"v":1,"seq":1,"ts":{NOW},"agents":[{{"pubkey":"{PUBKEY}","state":"running","since":{NOW}}},{{"pubkey":"{PUBKEY}","state":"stopped","since":{NOW}}}]}}"#
        );
        assert!(parse_host_status(&duplicate, NOW).is_err());

        let future_since = format!(
            r#"{{"v":1,"seq":1,"ts":{NOW},"agents":[{{"pubkey":"{PUBKEY}","state":"error","since":{}}}]}}"#,
            NOW + 1
        );
        assert!(parse_host_status(&future_since, NOW).is_err());
    }

    #[test]
    fn status_enforces_content_and_agent_caps() {
        let oversized = " ".repeat(HOST_STATUS_MAX_CONTENT_BYTES + 1);
        assert!(parse_host_status(&oversized, NOW).is_err());

        let agent = HostedAgentStatus {
            pubkey: PUBKEY.into(),
            state: HostedAgentState::Stopped,
            since: NOW,
        };
        let status = HostStatus {
            v: 1,
            seq: 1,
            ts: NOW,
            agents: vec![agent; HOST_STATUS_MAX_AGENTS + 1],
        };
        assert!(parse_host_status(&serde_json::to_string(&status).unwrap(), NOW).is_err());
    }
}
