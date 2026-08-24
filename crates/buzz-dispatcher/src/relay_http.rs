//! Minimal HTTP transport for the dispatcher.
//!
//! Mirrors the `buzz-cli` client contract: `POST /events` and `POST /query`
//! (the generic Nostr bridge endpoints) authenticated with NIP-98
//! (`Authorization: Nostr <base64 kind-27235 event>`), plus the optional
//! `x-auth-tag` header carrying the injected NIP-OA auth tag. Retries are
//! limited to definitive connect failures — ambiguous outcomes are surfaced
//! to the run loop, which is idempotent by request-ID pre-checks.

use std::time::Duration;

use base64::Engine as _;
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::DispatchError;

const RETRY_MAX_ATTEMPTS: u32 = 3;
/// TCP connect timeout (matches the buzz-cli default). A stalled connect
/// must never park the tick loop (M5).
const CONNECT_TIMEOUT_SECS: u64 = 15;
/// Per-request total timeout (matches the buzz-cli default). A live-but-wedged
/// relay must time out so the loop's error path and heartbeat liveness see it.
const REQUEST_TIMEOUT_SECS: u64 = 30;

/// Authenticated HTTP handle to one relay.
#[derive(Clone)]
pub struct RelayHttp {
    http: reqwest::Client,
    relay_url: String,
    keys: Keys,
    auth_tag: Option<String>,
}

/// Parsed `POST /events` receipt.
#[derive(Debug, Clone, PartialEq)]
pub struct SubmitReceipt {
    /// Event ID echoed by the relay.
    pub event_id: String,
    /// Whether the relay stored the event.
    pub accepted: bool,
    /// Relay message (reason when rejected).
    pub message: String,
}

impl RelayHttp {
    /// Build a transport handle. `relay_url` must be the http(s) base URL.
    ///
    /// Connect/request timeouts are always set (M5): a stalled connection
    /// parks the tick forever otherwise, and a live-but-wedged relay defeats
    /// KeepAlive liveness.
    pub fn new(
        relay_url: &str,
        keys: Keys,
        auth_tag: Option<String>,
    ) -> Result<Self, DispatchError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .map_err(|error| DispatchError::Other(format!("http client build failed: {error}")))?;
        Ok(Self {
            http,
            relay_url: relay_url.trim_end_matches('/').to_string(),
            keys,
            auth_tag,
        })
    }

    /// The service public key (hex) this handle signs with.
    pub fn pubkey_hex(&self) -> String {
        self.keys.public_key().to_hex()
    }

    /// The service keypair (for signing tracker/channel events).
    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    /// NIP-98 `Authorization` header for one request.
    fn auth_header(&self, method: &str, url: &str, body: &[u8]) -> Result<String, DispatchError> {
        let payload = hex::encode(Sha256::digest(body));
        let tags = vec![
            Tag::parse(["u", url]).map_err(tag_error)?,
            Tag::parse(["method", method]).map_err(tag_error)?,
            // Fresh nonce per attempt: the derived event ID differs each try,
            // keeping retries safe against the relay's replay guard.
            Tag::parse(["nonce", &Uuid::new_v4().to_string()]).map_err(tag_error)?,
            Tag::parse(["payload", &payload]).map_err(tag_error)?,
        ];
        let event = EventBuilder::new(Kind::Custom(27235), "")
            .tags(tags)
            .sign_with_keys(&self.keys)
            .map_err(|error| DispatchError::Identity(format!("NIP-98 signing failed: {error}")))?;
        let json = event.as_json();
        Ok(format!(
            "Nostr {}",
            base64::engine::general_purpose::STANDARD.encode(json.as_bytes())
        ))
    }

    fn with_auth_tag(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.auth_tag {
            Some(json) => req.header("x-auth-tag", json),
            None => req,
        }
    }

    async fn post(&self, path: &str, body: Vec<u8>) -> Result<String, DispatchError> {
        let url = format!("{}{}", self.relay_url, path);
        let mut last_error: Option<DispatchError> = None;
        for attempt in 0..RETRY_MAX_ATTEMPTS {
            let is_last = attempt == RETRY_MAX_ATTEMPTS - 1;
            let auth = self.auth_header("POST", &url, &body)?;
            let result = self
                .with_auth_tag(
                    self.http
                        .post(&url)
                        .header("Authorization", auth)
                        .header("Content-Type", "application/json")
                        .body(body.clone()),
                )
                .send()
                .await;
            match result {
                Err(error) if error.is_connect() && !is_last => {
                    // Definitively never reached the relay — safe to retry.
                    last_error = Some(DispatchError::Http(error));
                    tokio::time::sleep(std::time::Duration::from_millis(
                        u64::from(attempt + 1) * 100,
                    ))
                    .await;
                    continue;
                }
                Err(error) => return Err(DispatchError::Http(error)),
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await?;
                    if status.is_success() {
                        return Ok(text);
                    }
                    return Err(DispatchError::Relay(format!(
                        "relay returned {status} for {path}: {text}"
                    )));
                }
            }
        }
        Err(last_error.unwrap_or_else(|| DispatchError::Other("retry loop exited empty".into())))
    }

    /// Submit one signed event via `POST /events`; returns the raw relay
    /// response body (callers parse with [`parse_submit_receipt`]).
    pub async fn submit_event(&self, event: &Event) -> Result<String, DispatchError> {
        let body = serde_json::to_vec(event)?;
        self.post("/events", body).await
    }

    /// Run Nostr REQ filters via `POST /query`; returns the event array.
    ///
    /// Each element of `filters` must be ONE Nostr filter OBJECT; the bridge
    /// parses the body as `Vec<Filter>` (a top-level array of filter objects).
    /// Passing an array-wrapped filter here double-nests the body (`[[{...}]]`)
    /// and the relay answers 400 invalid filters (B1 regression guard: see
    /// [`query_body`] and its test).
    pub async fn query(&self, filters: &[Value]) -> Result<Vec<Value>, DispatchError> {
        let body = query_body(filters)?;
        let raw = self.post("/query", body).await?;
        let parsed: Vec<Value> = serde_json::from_str(&raw).map_err(|error| {
            DispatchError::Relay(format!("query response was not an event array: {error}"))
        })?;
        Ok(parsed)
    }
}

/// Serialize REQ filters into the exact `POST /query` body bytes.
///
/// The bridge contract is a top-level JSON array whose elements each
/// deserialize as one `nostr::Filter` — so `filters` must be filter objects,
/// never arrays. Kept separate from [`RelayHttp::query`] so the serialized
/// shape is unit-testable without a live relay.
pub fn query_body(filters: &[Value]) -> Result<Vec<u8>, DispatchError> {
    for filter in filters {
        if !filter.is_object() {
            return Err(DispatchError::Other(format!(
                "query filter must be a JSON object, got: {filter}"
            )));
        }
    }
    Ok(serde_json::to_vec(filters)?)
}

fn tag_error(error: impl std::fmt::Display) -> DispatchError {
    DispatchError::Other(format!("tag error: {error}"))
}

/// Parse one `POST /events` receipt. Accepts both `event_id` and `id`
/// spellings for the echoed event ID.
pub fn parse_submit_receipt(raw: &str) -> Result<SubmitReceipt, DispatchError> {
    let value: Value = serde_json::from_str(raw).map_err(|error| {
        DispatchError::Relay(format!("submit response was not JSON: {error}: {raw}"))
    })?;
    let event_id = value
        .get("event_id")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| DispatchError::Relay(format!("submit response missing event id: {raw}")))?
        .to_string();
    let accepted = value
        .get("accepted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(SubmitReceipt {
        event_id,
        accepted,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_header_is_nostr_base64_kind_27235() {
        let keys = Keys::generate();
        let relay = RelayHttp::new("http://localhost:3000/", keys.clone(), None).expect("build");
        let header = relay
            .auth_header("POST", "http://localhost:3000/events", b"{}")
            .expect("header");
        assert!(header.starts_with("Nostr "));
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(header.trim_start_matches("Nostr "))
            .expect("base64");
        let event: Event = serde_json::from_slice(&decoded).expect("event json");
        assert_eq!(event.kind, Kind::Custom(27235));
        let tags: Vec<Vec<String>> = event
            .tags
            .iter()
            .map(|tag| {
                tag.clone()
                    .to_vec()
                    .into_iter()
                    .map(|content| content.to_string())
                    .collect()
            })
            .collect();
        assert!(tags
            .iter()
            .any(|tag| tag[0] == "u" && tag[1] == "http://localhost:3000/events"));
        assert!(tags
            .iter()
            .any(|tag| tag[0] == "method" && tag[1] == "POST"));
        assert!(tags.iter().any(|tag| tag[0] == "nonce"));
        assert!(tags.iter().any(|tag| tag[0] == "payload"));
    }

    #[test]
    fn receipt_parser_accepts_both_id_spellings() {
        let a = parse_submit_receipt(r#"{"event_id":"abc123","accepted":true,"message":"stored"}"#)
            .expect("parse");
        assert_eq!(a.event_id, "abc123");
        assert!(a.accepted);
        assert_eq!(a.message, "stored");

        let b = parse_submit_receipt(r#"{"id":"def456","accepted":false,"message":"duplicate"}"#)
            .expect("parse");
        assert_eq!(b.event_id, "def456");
        assert!(!b.accepted);

        assert!(parse_submit_receipt(r#"{"accepted":true}"#).is_err());
        assert!(parse_submit_receipt("not json").is_err());
    }

    /// B1 regression guard: the `POST /query` body must be a top-level array
    /// of filter OBJECTS (`[{...}]`). A filter that is itself an array
    /// serializes as `[[{...}]]`, which the bridge rejects with 400 invalid
    /// filters — that shape made every relay query fail.
    #[test]
    fn query_body_is_flat_array_of_filter_objects() {
        let filter = serde_json::json!({ "kinds": [7], "since": 100 });
        let body = query_body(std::slice::from_ref(&filter)).expect("serialize");
        let parsed: Value = serde_json::from_slice(&body).expect("valid json");
        let array = parsed.as_array().expect("top-level array");
        assert_eq!(array.len(), 1);
        assert!(array[0].is_object(), "each element must be a filter object");
        assert!(!array[0].is_array(), "no double-nested filter array");
        assert_eq!(array[0]["kinds"], serde_json::json!([7]));
        assert_eq!(array[0]["since"], serde_json::json!(100));
    }

    #[test]
    fn query_body_rejects_non_object_filters() {
        let nested = serde_json::json!([{ "kinds": [7] }]);
        assert!(query_body(std::slice::from_ref(&nested)).is_err());
        assert!(query_body(&[serde_json::json!("not a filter")]).is_err());
    }
}
