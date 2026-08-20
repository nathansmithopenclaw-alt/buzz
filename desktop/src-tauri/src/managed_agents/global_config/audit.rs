//! Deterministic private audit channel for agent-management receipts.
//!
//! UUID construction matches starter channels: uuid5(namespace, "name:v1:…").
//! `relay_scope` is a canonical websocket identity so two devices of the same
//! owner on the same relay converge instead of minting near-miss channels.

/// Same namespace UUID as `commands/channels.rs` `STARTER_CHANNEL_NAMESPACE`.
const AGENT_MANAGEMENT_AUDIT_NAMESPACE: uuid::Uuid =
    uuid::uuid!("3ce33bea-8f09-5f1b-9c85-8a7d2659e6b0");

/// Normalize a relay URL into the uuid5 `relay_scope` token.
///
/// - `http`/`ws` → `ws`; everything else (`https`/`wss`/unknown) → `wss`
/// - host is lowercased
/// - default ports (80 for ws, 443 for wss) are omitted
/// - path, query, fragment, and trailing slash are dropped
pub fn canonical_relay_scope(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("wss://{trimmed}")
    };
    let Ok(url) = url::Url::parse(&candidate) else {
        return trimmed.trim_end_matches('/').to_ascii_lowercase();
    };
    let ws_scheme = match url.scheme() {
        "http" | "ws" => "ws",
        _ => "wss",
    };
    let Some(host) = url.host_str() else {
        return trimmed.trim_end_matches('/').to_ascii_lowercase();
    };
    let host = host.to_ascii_lowercase();
    match url.port() {
        Some(port) => format!("{ws_scheme}://{host}:{port}"),
        None => format!("{ws_scheme}://{host}"),
    }
}

/// Deterministic channel id for this owner on this relay.
pub fn agent_management_audit_channel_uuid(owner_hex: &str, relay_scope: &str) -> uuid::Uuid {
    let name = format!(
        "agent-management-audit:v1:{}:{}",
        owner_hex.trim().to_ascii_lowercase(),
        relay_scope
    );
    uuid::Uuid::new_v5(&AGENT_MANAGEMENT_AUDIT_NAMESPACE, name.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_relay_scope_converges_scheme_slash_port_and_case() {
        let expected = "wss://relay.example:8443";
        for input in [
            "wss://relay.example:8443",
            "wss://relay.example:8443/",
            "https://relay.example:8443",
            "https://relay.example:8443/",
            "WSS://Relay.Example:8443/",
        ] {
            assert_eq!(canonical_relay_scope(input), expected, "{input}");
        }
    }

    #[test]
    fn canonical_relay_scope_omits_default_ports_and_maps_http_to_ws() {
        assert_eq!(
            canonical_relay_scope("https://example.com"),
            "wss://example.com"
        );
        assert_eq!(
            canonical_relay_scope("wss://example.com:443"),
            "wss://example.com"
        );
        assert_eq!(
            canonical_relay_scope("http://localhost:3000/"),
            "ws://localhost:3000"
        );
        assert_eq!(
            canonical_relay_scope("ws://localhost:3000"),
            "ws://localhost:3000"
        );
    }

    #[test]
    fn audit_uuid_is_stable_for_owner_and_canonical_scope() {
        let owner = "aa".repeat(32);
        let a = agent_management_audit_channel_uuid(
            &owner,
            &canonical_relay_scope("wss://relay.example:8443/"),
        );
        let b = agent_management_audit_channel_uuid(
            &owner.to_ascii_uppercase(),
            &canonical_relay_scope("https://relay.example:8443"),
        );
        assert_eq!(a, b);
        let other_owner = agent_management_audit_channel_uuid(
            &"bb".repeat(32),
            &canonical_relay_scope("wss://relay.example:8443"),
        );
        assert_ne!(a, other_owner);
        let other_relay = agent_management_audit_channel_uuid(
            &owner,
            &canonical_relay_scope("wss://other.example:8443"),
        );
        assert_ne!(a, other_relay);
    }
}
