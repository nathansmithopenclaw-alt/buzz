//! NIP-AM: Agent Turn Metric — payload type and encrypt/decrypt helpers.
//!
//! One `kind:44200` event is published per completed agent turn. Its content
//! is a NIP-44 v2 ciphertext (agent key → owner pubkey) that decodes to an
//! [`AgentTurnMetricPayload`] JSON object.
//!
//! See `docs/nips/NIP-AM.md` for the full specification.

use nostr::{Event, Keys, PublicKey};
use serde::{Deserialize, Serialize};

use crate::observer::{
    decrypt_observer_payload, decrypt_observer_plaintext, encrypt_observer_payload,
    ObserverPayloadError,
};

// Re-export for callers that only need the error type.
pub use crate::observer::ObserverPayloadError as AgentTurnMetricError;

/// Token-usage counters for a single measurement window (one turn or cumulative).
///
/// All token fields are nullable — `None` means the harness did not report them,
/// NOT that the count was zero. See NIP-AM §Numeric validity and token semantics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenCounts {
    /// Input tokens (inclusive of cache reads/writes where applicable).
    pub input_tokens: Option<u64>,

    /// Output tokens.
    pub output_tokens: Option<u64>,

    /// Provider-reported total — NOT derived by summing input + output.
    /// `None` when the provider did not report a total.
    pub total_tokens: Option<u64>,

    /// Estimated cost in USD. Must be finite and non-negative when present.
    pub cost_usd: Option<f64>,

    /// Informational: cache-read tokens included in `input_tokens`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,

    /// Informational: cache-write tokens included in `input_tokens`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
}

/// Why a turn ended.
///
/// What one `turn` measurement actually covers.
///
/// An agent turn normally issues many model calls. Harnesses differ in what
/// they report per turn, and the two bases differ by orders of magnitude, so a
/// consumer that mixes them produces nonsense: on 2026-09-03 two publishers
/// with turn counts of the same order (81 and 36) reported 124.7M and 3.4M
/// `turn.inputTokens` respectively. That is the one observed instance this
/// codebase cites; `docs/nips/NIP-AM.md` carries it and the supporting
/// distribution, and the two must not drift apart.
///
/// Publishers SHOULD set `basis`. Consumers MUST NOT sum, average, or rank
/// across differing bases, and MUST treat an absent or unrecognized `basis` as
/// unknown — which is itself not comparable with either known value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricBasis {
    /// Counts accumulate across every model call the turn made. A turn's
    /// `inputTokens` may exceed the context window many times over.
    PerTurnSum,
    /// Counts describe a single model response. A turn that made many calls is
    /// reported at the magnitude of one, so these figures UNDERSTATE the turn.
    PerResponse,
}

/// Unrecognized values deserialize to `None` at the field, mirroring the
/// forward-compatibility rule for `stopReason`: a future basis must not fail
/// the whole payload, and must not be silently mistaken for a known one.
impl<'de> Deserialize<'de> for MetricBasis {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "per_turn_sum" => Ok(MetricBasis::PerTurnSum),
            "per_response" => Ok(MetricBasis::PerResponse),
            other => Err(serde::de::Error::custom(format!(
                "unrecognized basis: {other}"
            ))),
        }
    }
}

/// NIP-AM: consumers MUST treat unrecognized `stopReason` values as `Unknown`
/// and keep the token counts valid. Custom deserialization maps any unrecognized
/// string to `Unknown` instead of failing the whole payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Model reached a natural end-of-turn.
    EndTurn,
    /// Model hit the max-tokens limit.
    MaxTokens,
    /// Turn was cancelled by the owner or harness.
    Cancelled,
    /// Turn ended with an error.
    Error,
    /// Stop reason is unknown or unrecognized.
    Unknown,
}

impl<'de> Deserialize<'de> for StopReason {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(match s.as_str() {
            "end_turn" => StopReason::EndTurn,
            "max_tokens" => StopReason::MaxTokens,
            "cancelled" => StopReason::Cancelled,
            "error" => StopReason::Error,
            "unknown" => StopReason::Unknown,
            _ => StopReason::Unknown,
        })
    }
}

/// Why the ACP session this turn ran in had to be created.
///
/// NIP-AM: consumers MUST treat unrecognized `sessionCreationCause` values as
/// `Unknown` and keep the token counts valid, mirroring the `stopReason`
/// contract. Custom deserialization maps any unrecognized string to `Unknown`
/// instead of failing the whole payload.
///
/// Every variant except [`WorkerSpawn`](Self::WorkerSpawn) and
/// [`ScopeFirstTurn`](Self::ScopeFirstTurn) also names the teardown cause of
/// whatever session preceded this one in the same scope — those two are the
/// "no predecessor" cases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionCreationCause {
    /// The publisher process held no session state at all — a fresh or
    /// respawned worker. Nothing was cached anywhere in the process.
    WorkerSpawn,
    /// The worker already held sessions for other scopes; this scope simply
    /// had none yet (new channel, new thread).
    ScopeFirstTurn,
    /// Proactive rotation after a configured per-session turn limit.
    RotationTurnLimit,
    /// The provider ended a turn at a limit that invalidates the session
    /// (e.g. max tokens, max turn requests).
    ProviderLimit,
    /// The session owner explicitly asked for a rotation.
    OwnerRotate,
    /// The session owner cancelled, steered, or interrupted the in-flight
    /// turn, and the session was invalidated as part of that cleanup.
    OwnerInterrupt,
    /// A model switch required a new session to take effect.
    ModelSwitch,
    /// The agent's membership in the channel changed.
    MembershipChange,
    /// The adapter reported a runtime failure during the previous turn.
    AdapterFailure,
    /// The adapter process exited.
    AgentExited,
    /// The previous turn hit an idle or hard-cap timeout.
    TurnTimeout,
    /// Any other error path invalidated the previous session.
    Error,
    /// Cause is unknown or unrecognized.
    Unknown,
}

impl<'de> Deserialize<'de> for SessionCreationCause {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(match s.as_str() {
            "worker_spawn" => SessionCreationCause::WorkerSpawn,
            "scope_first_turn" => SessionCreationCause::ScopeFirstTurn,
            "rotation_turn_limit" => SessionCreationCause::RotationTurnLimit,
            "provider_limit" => SessionCreationCause::ProviderLimit,
            "owner_rotate" => SessionCreationCause::OwnerRotate,
            "owner_interrupt" => SessionCreationCause::OwnerInterrupt,
            "model_switch" => SessionCreationCause::ModelSwitch,
            "membership_change" => SessionCreationCause::MembershipChange,
            "adapter_failure" => SessionCreationCause::AdapterFailure,
            "agent_exited" => SessionCreationCause::AgentExited,
            "turn_timeout" => SessionCreationCause::TurnTimeout,
            "error" => SessionCreationCause::Error,
            _ => SessionCreationCause::Unknown,
        })
    }
}

/// Billing identity for a turn — present only when the publisher can prove
/// applicability from the actual endpoint and actually-requested model.
///
/// NIP-AM: this is OPTIONAL but NOT nullable. When present, `authority` and
/// `model` MUST be non-null strings; `cache_class` is omitted (not null) when
/// not applicable.
///
/// Consumers MUST treat omission as "price unknown" and MUST NOT infer a price
/// from the session `model` field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingIdentity {
    /// Registered billing-namespace identifier: exact lowercase hostname, no
    /// scheme, no path, no trailing slash. Registered values: `api.anthropic.com`,
    /// `api.openai.com`, `openrouter.ai`. Set extends only by NIP amendment.
    pub authority: String,

    /// The billable model identifier as resolved at request time — the
    /// actually-requested model, not the configured/session model alias.
    pub model: String,

    /// Cache-write class (e.g. `"ephemeral"`). Omitted when not applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_class: Option<String>,
}

/// Decrypted payload of a `kind:44200` Agent Turn Metric event.
/// nullable unless constrained by the NIP (e.g. `session_id` + `turn_seq`
/// are required whenever `cumulative` is present).
///
/// Consumers MUST ignore unknown fields (forward compatibility).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurnMetricPayload {
    /// Harness identifier (e.g. `"goose"`, `"buzz-agent"`). REQUIRED.
    pub harness: String,

    /// Model identifier as reported by the harness, or `None` if unknown.
    pub model: Option<String>,

    /// Channel UUID the turn served, encrypted inside the payload.
    pub channel_id: Option<String>,

    /// Session identifier. REQUIRED when `cumulative` is present.
    pub session_id: Option<String>,

    /// Turn identifier (harness-internal).
    pub turn_id: Option<String>,

    /// Monotonically increasing per-session sequence number.
    /// REQUIRED when `cumulative` is present; strictly increasing within one
    /// `session_id`. A publisher restart that loses the counter MUST start a
    /// new `session_id`.
    pub turn_seq: Option<u64>,

    /// RFC 3339 timestamp (end-of-turn). REQUIRED.
    pub timestamp: String,

    /// Usage for this turn (computed delta). Null fields mean not reported.
    pub turn: Option<TokenCounts>,

    /// Session-cumulative usage as reported at end of this turn.
    pub cumulative: Option<TokenCounts>,

    /// `false` when the publisher could not observe the previous cumulative
    /// baseline (e.g. harness restart mid-session), making `turn` unreliable.
    /// Defaults to `true` on the wire when not explicitly set.
    #[serde(default = "default_delta_reliable")]
    pub delta_reliable: bool,

    /// Why the turn ended. Unrecognized values MUST be treated as `Unknown`.
    pub stop_reason: Option<StopReason>,

    /// Billing identity, present only when the publisher can prove it from the
    /// actual endpoint (official provider API) and the actually-requested model.
    ///
    /// Omit (never null) when applicability cannot be proven. Consumers MUST
    /// treat omission as "price unknown".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing_identity: Option<PricingIdentity>,

    /// Context tokens occupied by the conversation at the end of this turn, as
    /// reported by the harness. Omitted (never null, never a fabricated zero)
    /// when the harness does not report context occupancy.
    ///
    /// This is a *state* reading, not a usage delta — it is not summable
    /// across turns and MUST NOT be added to any token total.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,

    /// Size of the model's context window for this turn, as reported by the
    /// harness. Omitted when unreported. Same non-summable contract as
    /// `context_tokens`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,

    /// 1-based delivery attempt of the work that triggered this turn: `1` on
    /// first delivery, `2` on the first retry, and so on. Omitted when the
    /// publisher has no attempt notion for this turn (e.g. a self-scheduled
    /// heartbeat that is never retried).
    ///
    /// Attribution only. A value above `1` does NOT prove the provider ran the
    /// earlier attempt — a retry may follow a failure that never reached the
    /// model. Consumers MUST NOT treat `attempt > 1` as duplicated token spend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,

    /// `true` when every event that triggered this turn predates the
    /// publisher's own startup — i.e. the turn happened only because the
    /// publisher restarted and replayed a bounded historical window. Omitted
    /// when the publisher has no replay notion for this turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_replay: Option<bool>,

    /// What the `turn` counts measure — see [`MetricBasis`]. Omitted when the
    /// publisher cannot determine it; an unrecognized wire value also
    /// deserializes to `None`. Absent means unknown, NOT comparable-by-default.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_basis"
    )]
    pub basis: Option<MetricBasis>,

    /// Why the session this turn ran in had to be created. Constant for every
    /// turn of one `sessionId`, so consumers can attribute a session's
    /// cache-write cost to its cause and see how far it amortized. Omitted
    /// when the publisher cannot determine it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_creation_cause: Option<SessionCreationCause>,
}

/// Accept an unrecognized `basis` as `None` rather than failing the payload.
/// A metric event whose only unknown is a future basis label is still worth
/// keeping — the counts are intact and the basis is honestly unknown.
fn deserialize_optional_basis<'de, D>(deserializer: D) -> Result<Option<MetricBasis>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    Ok(match raw.as_deref() {
        Some("per_turn_sum") => Some(MetricBasis::PerTurnSum),
        Some("per_response") => Some(MetricBasis::PerResponse),
        _ => None,
    })
}

fn default_delta_reliable() -> bool {
    true
}

impl AgentTurnMetricPayload {
    /// Validate numeric constraints from NIP-AM §Numeric validity.
    ///
    /// Returns `Err` when any `cost_usd` field (in `turn` or `cumulative`) is
    /// present but negative or non-finite (NaN or infinity). Token counts are
    /// typed as `Option<u64>` and therefore cannot be negative by construction.
    pub fn validate(&self) -> Result<(), ObserverPayloadError> {
        fn check_cost(cost: Option<f64>, field: &str) -> Result<(), ObserverPayloadError> {
            if let Some(c) = cost {
                if !c.is_finite() || c < 0.0 {
                    return Err(ObserverPayloadError::InvalidPayload(format!(
                        "{field} must be finite and non-negative (got {c})"
                    )));
                }
            }
            Ok(())
        }
        if let Some(t) = &self.turn {
            check_cost(t.cost_usd, "turn.costUsd")?;
        }
        if let Some(c) = &self.cumulative {
            check_cost(c.cost_usd, "cumulative.costUsd")?;
        }
        Ok(())
    }
}

/// Encrypt an [`AgentTurnMetricPayload`] into a NIP-44 v2 ciphertext string
/// using the agent's key pair and the owner's public key.
///
/// Returns `Err(ObserverPayloadError::InvalidPayload)` if any `cost_usd` field
/// is negative or non-finite (NaN/inf), in accordance with NIP-AM §Numeric
/// validity.
///
/// This is the content field of a `kind:44200` event.
pub fn encrypt_agent_turn_metric(
    agent_keys: &Keys,
    owner_pubkey: &PublicKey,
    payload: &AgentTurnMetricPayload,
) -> Result<String, ObserverPayloadError> {
    payload.validate()?;
    encrypt_observer_payload(agent_keys, owner_pubkey, payload)
}

/// Decrypt and deserialize an [`AgentTurnMetricPayload`] from a `kind:44200` event.
///
/// `recipient_keys` is the owner's key pair.
///
/// Returns `Err(ObserverPayloadError::InvalidPayload)` if the decrypted payload
/// fails numeric validation (e.g. negative or non-finite `costUsd`), mirroring
/// the fail-closed contract of [`encrypt_agent_turn_metric`].
pub fn decrypt_agent_turn_metric(
    recipient_keys: &Keys,
    event: &Event,
) -> Result<AgentTurnMetricPayload, ObserverPayloadError> {
    let payload: AgentTurnMetricPayload = decrypt_observer_payload(recipient_keys, event)?;
    payload.validate()?;
    Ok(payload)
}

/// Decrypt a `kind:44200` event, returning both the validated payload and the
/// exact plaintext the publisher sent.
///
/// A store that keeps a decrypted copy must keep the publisher's bytes, not a
/// re-serialization through this build's struct: NIP-AM requires consumers to
/// ignore unknown fields, and re-serializing silently *deletes* them. Because
/// an archive of these events is append-only, a build that predates a new field
/// would otherwise destroy it permanently — the ciphertext is not retained, so
/// no later upgrade could recover it.
///
/// Same validation contract as [`decrypt_agent_turn_metric`]: the plaintext is
/// returned only for a payload that deserialized and passed numeric validation,
/// so a caller can never persist an unvalidated blob.
pub fn decrypt_agent_turn_metric_with_plaintext(
    recipient_keys: &Keys,
    event: &Event,
) -> Result<(AgentTurnMetricPayload, String), ObserverPayloadError> {
    let plaintext = decrypt_observer_plaintext(recipient_keys, event)?;
    let payload: AgentTurnMetricPayload = serde_json::from_str(&plaintext)?;
    payload.validate()?;
    Ok((payload, plaintext))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Kind, Tag};

    fn sample_payload() -> AgentTurnMetricPayload {
        AgentTurnMetricPayload {
            harness: "goose".to_string(),
            model: Some("claude-sonnet-4-5".to_string()),
            channel_id: Some("12345678-1234-1234-1234-123456789abc".to_string()),
            session_id: Some("sess-abc".to_string()),
            turn_id: Some("turn-1".to_string()),
            turn_seq: Some(1),
            timestamp: "2026-07-01T20:11:03.213Z".to_string(),
            turn: Some(TokenCounts {
                input_tokens: Some(1234),
                output_tokens: Some(567),
                total_tokens: Some(1801),
                cost_usd: Some(0.0123),
                cache_read_tokens: None,
                cache_write_tokens: None,
            }),
            cumulative: Some(TokenCounts {
                input_tokens: Some(45210),
                output_tokens: Some(9876),
                total_tokens: Some(55086),
                cost_usd: Some(0.41),
                cache_read_tokens: None,
                cache_write_tokens: None,
            }),
            delta_reliable: true,
            stop_reason: Some(StopReason::EndTurn),
            pricing_identity: None,
            context_tokens: None,
            context_window: None,
            attempt: None,
            is_replay: None,
            session_creation_cause: None,
            basis: None,
        }
    }

    #[test]
    fn round_trip_encrypt_decrypt() {
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();

        let payload = sample_payload();
        let ciphertext = encrypt_agent_turn_metric(&agent_keys, &owner_keys.public_key(), &payload)
            .expect("encrypt");

        // Build a minimal event envelope so decrypt_observer_payload can use event.pubkey.
        let event = EventBuilder::new(Kind::Custom(44200), ciphertext)
            .tags([
                Tag::parse(["p", &owner_keys.public_key().to_hex()]).unwrap(),
                Tag::parse(["agent", &agent_keys.public_key().to_hex()]).unwrap(),
            ])
            .sign_with_keys(&agent_keys)
            .expect("sign");

        let decoded = decrypt_agent_turn_metric(&owner_keys, &event).expect("decrypt");

        assert_eq!(decoded, payload);
    }

    #[test]
    fn wrong_key_decrypt_fails() {
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let wrong_keys = Keys::generate();

        let payload = sample_payload();
        let ciphertext = encrypt_agent_turn_metric(&agent_keys, &owner_keys.public_key(), &payload)
            .expect("encrypt");

        let event = EventBuilder::new(Kind::Custom(44200), ciphertext)
            .tags([
                Tag::parse(["p", &owner_keys.public_key().to_hex()]).unwrap(),
                Tag::parse(["agent", &agent_keys.public_key().to_hex()]).unwrap(),
            ])
            .sign_with_keys(&agent_keys)
            .expect("sign");

        let result = decrypt_agent_turn_metric(&wrong_keys, &event);
        assert!(result.is_err(), "expected decrypt error with wrong key");
    }

    #[test]
    fn delta_reliable_defaults_to_true_when_absent() {
        let json = r#"{"harness":"goose","timestamp":"2026-07-01T20:11:03Z"}"#;
        let payload: AgentTurnMetricPayload = serde_json::from_str(json).expect("parse");
        assert!(
            payload.delta_reliable,
            "deltaReliable should default to true"
        );
        assert_eq!(payload.is_replay, None);
    }

    #[test]
    fn stop_reason_round_trips() {
        for (variant, json_val) in [
            (StopReason::EndTurn, "\"end_turn\""),
            (StopReason::MaxTokens, "\"max_tokens\""),
            (StopReason::Cancelled, "\"cancelled\""),
            (StopReason::Error, "\"error\""),
            (StopReason::Unknown, "\"unknown\""),
        ] {
            let serialized = serde_json::to_string(&variant).unwrap();
            assert_eq!(serialized, json_val);
            let deserialized: StopReason = serde_json::from_str(json_val).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn session_creation_cause_round_trips() {
        for (variant, json_val) in [
            (SessionCreationCause::WorkerSpawn, "\"worker_spawn\""),
            (SessionCreationCause::ScopeFirstTurn, "\"scope_first_turn\""),
            (
                SessionCreationCause::RotationTurnLimit,
                "\"rotation_turn_limit\"",
            ),
            (SessionCreationCause::ProviderLimit, "\"provider_limit\""),
            (SessionCreationCause::OwnerRotate, "\"owner_rotate\""),
            (SessionCreationCause::OwnerInterrupt, "\"owner_interrupt\""),
            (SessionCreationCause::ModelSwitch, "\"model_switch\""),
            (
                SessionCreationCause::MembershipChange,
                "\"membership_change\"",
            ),
            (SessionCreationCause::AdapterFailure, "\"adapter_failure\""),
            (SessionCreationCause::AgentExited, "\"agent_exited\""),
            (SessionCreationCause::TurnTimeout, "\"turn_timeout\""),
            (SessionCreationCause::Error, "\"error\""),
            (SessionCreationCause::Unknown, "\"unknown\""),
        ] {
            let serialized = serde_json::to_string(&variant).unwrap();
            assert_eq!(serialized, json_val);
            let deserialized: SessionCreationCause = serde_json::from_str(json_val).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn unknown_session_creation_cause_maps_to_unknown_not_error() {
        // Same forward-compat contract as stopReason: a cause this build has
        // never heard of must not reject the payload or void its counters.
        let json = r#"{
            "harness": "buzz-acp",
            "timestamp": "2026-07-01T20:11:03Z",
            "sessionCreationCause": "quantum_realignment",
            "turn": { "inputTokens": 1234, "outputTokens": 567, "totalTokens": null, "costUsd": null }
        }"#;
        let payload: AgentTurnMetricPayload =
            serde_json::from_str(json).expect("payload with future cause must parse");
        assert_eq!(
            payload.session_creation_cause,
            Some(SessionCreationCause::Unknown)
        );
        let turn = payload.turn.expect("turn must be present");
        assert_eq!(turn.input_tokens, Some(1234));
    }

    #[test]
    fn provenance_fields_are_omitted_not_null_when_absent() {
        // Absence must be omission, never `null` and never a fabricated zero —
        // the same contract the cache fields already hold. A `"attempt":0`
        // or `"contextTokens":0` on the wire would be read as a real reading.
        let payload = sample_payload();
        assert!(payload.attempt.is_none(), "fixture must leave these unset");
        let json = serde_json::to_string(&payload).unwrap();
        for field in [
            "contextTokens",
            "contextWindow",
            "attempt",
            "isReplay",
            "sessionCreationCause",
        ] {
            assert!(
                !json.contains(field),
                "{field} must be omitted entirely when unset, got: {json}"
            );
        }
    }

    #[test]
    fn provenance_fields_round_trip_through_encryption() {
        // Positive control for the omission test above: when the publisher DOES
        // set them, all five survive encrypt → decrypt with their exact values,
        // including the `attempt: 1` / `isReplay: false` cases that a
        // skip-if-falsy bug would silently drop.
        let mut payload = sample_payload();
        payload.context_tokens = Some(145_213);
        payload.context_window = Some(200_000);
        payload.attempt = Some(1);
        payload.is_replay = Some(false);
        payload.session_creation_cause = Some(SessionCreationCause::RotationTurnLimit);

        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let ciphertext = encrypt_agent_turn_metric(&agent_keys, &owner_keys.public_key(), &payload)
            .expect("encrypt");
        let event = EventBuilder::new(Kind::Custom(44200), ciphertext)
            .tags([
                Tag::parse(["p", &owner_keys.public_key().to_hex()]).unwrap(),
                Tag::parse(["agent", &agent_keys.public_key().to_hex()]).unwrap(),
            ])
            .sign_with_keys(&agent_keys)
            .expect("sign");

        let decoded = decrypt_agent_turn_metric(&owner_keys, &event).expect("decrypt");
        assert_eq!(decoded, payload);
        assert_eq!(decoded.attempt, Some(1));
        assert_eq!(decoded.is_replay, Some(false));
        assert_eq!(decoded.context_tokens, Some(145_213));
        assert_eq!(decoded.context_window, Some(200_000));
        assert_eq!(
            decoded.session_creation_cause,
            Some(SessionCreationCause::RotationTurnLimit)
        );
    }

    #[test]
    fn payload_without_provenance_fields_still_parses() {
        // An older publisher emits none of the new fields. A new consumer must
        // read that payload unchanged, with every new field absent.
        let json = r#"{
            "harness": "goose",
            "model": null,
            "channelId": null,
            "sessionId": "s1",
            "turnId": null,
            "turnSeq": 1,
            "timestamp": "2026-07-01T20:11:03Z",
            "turn": { "inputTokens": 10, "outputTokens": 20, "totalTokens": 30, "costUsd": 0.01 },
            "cumulative": null,
            "deltaReliable": true,
            "stopReason": "end_turn"
        }"#;
        let payload: AgentTurnMetricPayload = serde_json::from_str(json).expect("parse");
        assert_eq!(payload.context_tokens, None);
        assert_eq!(payload.context_window, None);
        assert_eq!(payload.attempt, None);
        assert_eq!(payload.is_replay, None);
        assert_eq!(payload.session_creation_cause, None);
        assert_eq!(payload.turn.and_then(|t| t.input_tokens), Some(10));
    }

    #[test]
    fn null_token_counts_round_trip() {
        // Verify that None fields serialize to `null` (not absent), as required
        // by the NIP — consumers must distinguish "not reported" from "zero".
        let counts = TokenCounts {
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            cost_usd: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        };
        let json = serde_json::to_string(&counts).unwrap();
        // cache_* are skip_serializing_if = None, others serialize as null
        assert!(json.contains("\"inputTokens\":null"));
        assert!(json.contains("\"outputTokens\":null"));
        let back: TokenCounts = serde_json::from_str(&json).unwrap();
        assert_eq!(back, counts);
    }

    // ── NIP-AM `basis` wire contract ────────────────────────────────────────

    /// The wire strings are the contract. `agent_metric_index` stores this
    /// value verbatim and the desktop projection groups on it, so a rename here
    /// silently splits one basis into two and re-enables the mixed-basis
    /// blanking for a single-runtime fleet.
    #[test]
    fn metric_basis_serializes_to_its_documented_wire_strings() {
        assert_eq!(
            serde_json::to_string(&MetricBasis::PerTurnSum).unwrap(),
            r#""per_turn_sum""#,
        );
        assert_eq!(
            serde_json::to_string(&MetricBasis::PerResponse).unwrap(),
            r#""per_response""#,
        );
    }

    /// Both documented values must round-trip through the payload, which is the
    /// real path: `basis` is read via `deserialize_optional_basis`, not the
    /// enum's own `Deserialize`.
    #[test]
    fn basis_round_trips_through_the_payload() {
        for (wire, expected) in [
            ("per_turn_sum", MetricBasis::PerTurnSum),
            ("per_response", MetricBasis::PerResponse),
        ] {
            let json = format!(
                r#"{{
                    "harness": "claude-agent-acp",
                    "timestamp": "2026-09-03T20:11:03Z",
                    "basis": "{wire}",
                    "turn": {{ "inputTokens": 10, "outputTokens": 2, "totalTokens": 12, "costUsd": null }}
                }}"#
            );
            let payload: AgentTurnMetricPayload =
                serde_json::from_str(&json).expect("payload with a known basis must parse");
            assert_eq!(
                payload.basis,
                Some(expected),
                "basis {wire} must round-trip"
            );
        }
    }

    /// A future basis must read as absent rather than failing the payload or
    /// being mistaken for a known value — the same forward-compatibility rule
    /// as `stopReason`. Absent means "not comparable", which is the safe
    /// reading; silently adopting a known value would be the unsafe one.
    #[test]
    fn unrecognized_basis_reads_as_absent_and_keeps_the_counts() {
        let json = r#"{
            "harness": "some-future-adapter",
            "timestamp": "2026-09-03T20:11:03Z",
            "basis": "per_epoch_sum",
            "turn": {
                "inputTokens": 1234,
                "outputTokens": 567,
                "totalTokens": 1801,
                "costUsd": null
            }
        }"#;
        let payload: AgentTurnMetricPayload =
            serde_json::from_str(json).expect("payload with a future basis must parse");
        assert_eq!(
            payload.basis, None,
            "an unrecognized basis must read as absent, never as a known value",
        );
        let turn = payload.turn.expect("turn must survive an unknown basis");
        assert_eq!(turn.input_tokens, Some(1234));
        assert_eq!(turn.output_tokens, Some(567));
    }

    /// An omitted `basis` is absent, and an absent basis is not serialized back
    /// out — a publisher that declared nothing must not appear to have declared
    /// something on re-emission.
    #[test]
    fn absent_basis_stays_absent_in_both_directions() {
        let json = r#"{
            "harness": "goose",
            "timestamp": "2026-09-03T20:11:03Z",
            "turn": { "inputTokens": 1, "outputTokens": 1, "totalTokens": 2, "costUsd": null }
        }"#;
        let payload: AgentTurnMetricPayload =
            serde_json::from_str(json).expect("payload without basis must parse");
        assert_eq!(payload.basis, None);
        let back = serde_json::to_value(&payload).expect("must re-serialize");
        assert!(
            back.get("basis").is_none(),
            "an absent basis must be omitted, not emitted as null",
        );
    }

    #[test]
    fn unknown_stop_reason_maps_to_unknown_not_error() {
        // NIP-AM: consumers MUST treat unrecognized stopReason values as Unknown;
        // the token counts remain valid and the whole payload must not be rejected.
        let json = r#"{
            "harness": "goose",
            "timestamp": "2026-07-01T20:11:03Z",
            "stopReason": "tool_limit",
            "turn": {
                "inputTokens": 1234,
                "outputTokens": 567,
                "totalTokens": 1801,
                "costUsd": null
            }
        }"#;
        let payload: AgentTurnMetricPayload =
            serde_json::from_str(json).expect("payload with future stopReason must parse");
        assert_eq!(
            payload.stop_reason,
            Some(StopReason::Unknown),
            "unrecognized stopReason must map to Unknown"
        );
        // Token counts must be preserved.
        let turn = payload.turn.expect("turn must be present");
        assert_eq!(turn.input_tokens, Some(1234));
        assert_eq!(turn.output_tokens, Some(567));
        assert_eq!(turn.total_tokens, Some(1801));
    }

    // ── validate() — negative / non-finite costUsd ─────────────────────────

    fn make_payload_with_turn_cost(cost: Option<f64>) -> AgentTurnMetricPayload {
        AgentTurnMetricPayload {
            harness: "test".to_string(),
            model: None,
            channel_id: None,
            session_id: None,
            turn_id: None,
            turn_seq: None,
            timestamp: "2026-07-01T00:00:00Z".to_string(),
            turn: Some(TokenCounts {
                input_tokens: Some(100),
                output_tokens: Some(50),
                total_tokens: None,
                cost_usd: cost,
                cache_read_tokens: None,
                cache_write_tokens: None,
            }),
            cumulative: None,
            delta_reliable: true,
            stop_reason: None,
            pricing_identity: None,
            context_tokens: None,
            context_window: None,
            attempt: None,
            is_replay: None,
            session_creation_cause: None,
            basis: None,
        }
    }

    fn make_payload_with_cumulative_cost(cost: Option<f64>) -> AgentTurnMetricPayload {
        AgentTurnMetricPayload {
            harness: "test".to_string(),
            model: None,
            channel_id: None,
            session_id: None,
            turn_id: None,
            turn_seq: None,
            timestamp: "2026-07-01T00:00:00Z".to_string(),
            turn: None,
            cumulative: Some(TokenCounts {
                input_tokens: Some(500),
                output_tokens: Some(200),
                total_tokens: None,
                cost_usd: cost,
                cache_read_tokens: None,
                cache_write_tokens: None,
            }),
            delta_reliable: true,
            stop_reason: None,
            pricing_identity: None,
            context_tokens: None,
            context_window: None,
            attempt: None,
            is_replay: None,
            session_creation_cause: None,
            basis: None,
        }
    }

    #[test]
    fn validate_rejects_negative_turn_cost() {
        let payload = make_payload_with_turn_cost(Some(-0.001));
        assert!(
            matches!(
                payload.validate(),
                Err(ObserverPayloadError::InvalidPayload(_))
            ),
            "negative turn.costUsd must be rejected"
        );
    }

    #[test]
    fn validate_rejects_nan_turn_cost() {
        let payload = make_payload_with_turn_cost(Some(f64::NAN));
        assert!(
            matches!(
                payload.validate(),
                Err(ObserverPayloadError::InvalidPayload(_))
            ),
            "NaN turn.costUsd must be rejected"
        );
    }

    #[test]
    fn validate_rejects_infinite_turn_cost() {
        let payload = make_payload_with_turn_cost(Some(f64::INFINITY));
        assert!(
            matches!(
                payload.validate(),
                Err(ObserverPayloadError::InvalidPayload(_))
            ),
            "infinite turn.costUsd must be rejected"
        );
    }

    #[test]
    fn validate_rejects_negative_cumulative_cost() {
        let payload = make_payload_with_cumulative_cost(Some(-1.0));
        assert!(
            matches!(
                payload.validate(),
                Err(ObserverPayloadError::InvalidPayload(_))
            ),
            "negative cumulative.costUsd must be rejected"
        );
    }

    #[test]
    fn validate_accepts_finite_non_negative_cost() {
        // Zero, small, and larger values are all valid.
        for cost in [0.0_f64, 0.001, 1.0, 999.99] {
            let payload = make_payload_with_turn_cost(Some(cost));
            assert!(payload.validate().is_ok(), "cost {cost} should be accepted");
        }
    }

    #[test]
    fn validate_accepts_absent_cost() {
        let payload = make_payload_with_turn_cost(None);
        assert!(
            payload.validate().is_ok(),
            "absent costUsd must be accepted"
        );
    }

    #[test]
    fn encrypt_agent_turn_metric_rejects_negative_cost() {
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let payload = make_payload_with_turn_cost(Some(-0.5));
        let result = encrypt_agent_turn_metric(&agent_keys, &owner_keys.public_key(), &payload);
        assert!(
            matches!(result, Err(ObserverPayloadError::InvalidPayload(_))),
            "encrypt must reject payload with negative costUsd"
        );
    }

    #[test]
    fn decrypt_agent_turn_metric_rejects_negative_cost_bypassing_encrypt() {
        // Regression: a raw/misbehaving agent can persist a syntactically valid
        // NIP-44 payload with costUsd: -1 by calling encrypt_observer_payload
        // directly (bypassing the validating encrypt_agent_turn_metric helper).
        // decrypt_agent_turn_metric must reject it symmetrically.
        use crate::observer::encrypt_observer_payload;

        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();

        // Build a payload with negative costUsd and encrypt via the lower-level
        // path, bypassing encrypt_agent_turn_metric's validate() call.
        let bad_payload = make_payload_with_turn_cost(Some(-1.0));
        let ciphertext =
            encrypt_observer_payload(&agent_keys, &owner_keys.public_key(), &bad_payload)
                .expect("lower-level encrypt should succeed without validation");

        let event = EventBuilder::new(Kind::Custom(44200), ciphertext)
            .tags([
                Tag::parse(["p", &owner_keys.public_key().to_hex()]).unwrap(),
                Tag::parse(["agent", &agent_keys.public_key().to_hex()]).unwrap(),
            ])
            .sign_with_keys(&agent_keys)
            .expect("sign");

        let result = decrypt_agent_turn_metric(&owner_keys, &event);
        assert!(
            matches!(result, Err(ObserverPayloadError::InvalidPayload(_))),
            "decrypt must reject a payload with negative costUsd even when \
             encrypted via the lower-level path"
        );
    }
}
