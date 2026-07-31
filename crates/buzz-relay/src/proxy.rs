//! Trusted reverse-proxy scheme reconstruction.
//!
//! The request `Host` remains authoritative for tenant selection. This module
//! only reconstructs whether the public-facing transport was cleartext or TLS,
//! and only honors `X-Forwarded-Proto` when the TCP peer is in the explicit
//! operator allowlist. An empty allowlist is the fail-closed default.

use std::net::{IpAddr, SocketAddr};

use axum::http::HeaderMap;
use ipnet::IpNet;

/// Public-facing request scheme after the trusted-proxy boundary is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicScheme {
    /// Cleartext HTTP / WebSocket.
    Http,
    /// TLS HTTP / WebSocket.
    Https,
}

impl PublicScheme {
    /// Derive the deployment fallback from its configured relay URL.
    pub fn from_relay_url(relay_url: &str) -> Self {
        if relay_url.trim_start().starts_with("wss://")
            || relay_url.trim_start().starts_with("https://")
        {
            Self::Https
        } else {
            Self::Http
        }
    }

    /// HTTP spelling used by NIP-98 and public links.
    pub const fn http(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }

    /// WebSocket spelling used by NIP-42 and relay advertisements.
    pub const fn websocket(self) -> &'static str {
        match self {
            Self::Http => "ws",
            Self::Https => "wss",
        }
    }
}

/// Returns the public scheme for one request.
///
/// `X-Forwarded-Proto` is accepted only when all of these hold:
///
/// - a real TCP peer address is available;
/// - that peer IP is inside `trusted_proxies`;
/// - exactly one header value is present; and
/// - the value is exactly `http` or `https` (ASCII case-insensitive).
///
/// Otherwise the configured deployment scheme is retained. Forwarded host
/// headers are deliberately never consulted; row-zero host binding remains on
/// the actual request `Host` authority.
pub fn public_scheme(
    relay_url: &str,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    trusted_proxies: &[IpNet],
) -> PublicScheme {
    let fallback = PublicScheme::from_relay_url(relay_url);
    let Some(peer) = peer else {
        return fallback;
    };
    if !trusted_proxies
        .iter()
        .any(|network| network.contains(&peer.ip()))
    {
        return fallback;
    }

    let mut values = headers.get_all("x-forwarded-proto").iter();
    let Some(value) = values.next() else {
        return fallback;
    };
    if values.next().is_some() {
        return fallback;
    }
    let Ok(value) = value.to_str() else {
        return fallback;
    };
    match value.trim() {
        value if value.eq_ignore_ascii_case("http") => PublicScheme::Http,
        value if value.eq_ignore_ascii_case("https") => PublicScheme::Https,
        _ => fallback,
    }
}

/// Parses one trusted-proxy entry as an IP address or CIDR network.
pub(crate) fn parse_trusted_proxy(value: &str) -> Result<IpNet, String> {
    let value = value.trim();
    if let Ok(network) = value.parse::<IpNet>() {
        return Ok(network);
    }
    let address = value
        .parse::<IpAddr>()
        .map_err(|_| format!("invalid IP address or CIDR: {value:?}"))?;
    Ok(IpNet::from(address))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    fn headers(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-proto",
            HeaderValue::from_str(value).expect("header"),
        );
        headers
    }

    #[test]
    fn forwarded_proto_requires_an_explicitly_trusted_peer() {
        let trusted = ["127.0.0.1".parse::<IpAddr>().unwrap().into()];
        let peer = Some("127.0.0.1:41000".parse().unwrap());
        assert_eq!(
            public_scheme("ws://relay", &headers("https"), peer, &trusted),
            PublicScheme::Https
        );
        assert_eq!(
            public_scheme(
                "ws://relay",
                &headers("https"),
                Some("203.0.113.9:41000".parse().unwrap()),
                &trusted,
            ),
            PublicScheme::Http
        );
        assert_eq!(
            public_scheme("ws://relay", &headers("https"), None, &trusted),
            PublicScheme::Http
        );
    }

    #[test]
    fn malformed_or_ambiguous_forwarded_proto_falls_back() {
        let trusted = ["127.0.0.1".parse::<IpAddr>().unwrap().into()];
        let peer = Some("127.0.0.1:41000".parse().unwrap());
        assert_eq!(
            public_scheme("ws://relay", &headers("https,http"), peer, &trusted),
            PublicScheme::Http
        );

        let mut duplicate = headers("https");
        duplicate.append("x-forwarded-proto", HeaderValue::from_static("http"));
        assert_eq!(
            public_scheme("ws://relay", &duplicate, peer, &trusted),
            PublicScheme::Http
        );
    }

    #[test]
    fn trusted_proxy_entries_accept_addresses_and_cidrs() {
        assert_eq!(
            parse_trusted_proxy("127.0.0.1").unwrap(),
            "127.0.0.1/32".parse().unwrap()
        );
        assert_eq!(
            parse_trusted_proxy("10.0.0.0/8").unwrap(),
            "10.0.0.0/8".parse().unwrap()
        );
        assert!(parse_trusted_proxy("loopback").is_err());
    }
}
