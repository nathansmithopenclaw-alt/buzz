//! Provider-neutral read contract for shared Buzz Memory.
//!
//! Buzz owns authentication, output-context authorization, and receipts. A
//! provider owns its storage and truth semantics. The bundled [`NaiveProvider`]
//! is deliberately small and non-governance: it exists so upstream deployments
//! can exercise the protocol without adopting an external policy engine.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

/// Stable identifier for the bundled provider implementation.
pub const NAIVE_PROVIDER_ID: &str = "buzz-naive-v1";

/// Maximum result count accepted by the provider contract.
pub const MAX_QUERY_LIMIT: usize = 100;

/// Privacy grade of a memory record or effective read scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Clearance {
    /// Visible to every workspace member.
    WorkspacePublic,
    /// Visible only when both caller and output context permit restricted data.
    Restricted,
}

/// Explicit output context for a memory read. Context-less reads do not exist.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutputContext {
    /// A stream or forum channel.
    Channel { id: Uuid },
    /// A direct-message channel.
    Dm { id: Uuid },
    /// The workspace-wide public projection.
    WorkspacePublic,
}

impl OutputContext {
    /// Return the channel-like identifier, if this context has one.
    pub fn channel_id(&self) -> Option<Uuid> {
        match self {
            Self::Channel { id } | Self::Dm { id } => Some(*id),
            Self::WorkspacePublic => None,
        }
    }
}

/// Relay-derived authorization scope supplied to a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadScope {
    /// Authenticated Nostr pubkey as lowercase hex.
    pub pubkey: String,
    /// Community resolved from the request Host header.
    pub community_id: Uuid,
    /// Destination in which the provider output will be used.
    pub context: OutputContext,
    /// Channel IDs derived by the relay, never accepted from the client.
    pub accessible_channel_ids: Vec<Uuid>,
    /// Maximum grade the caller may read.
    pub caller_clearance: Clearance,
    /// Maximum grade safe for the output context.
    pub output_clearance: Clearance,
    /// Effective policy revision used for the read.
    pub policy_id: String,
}

/// Mandatory authorization identity for any read-plane cache entry.
///
/// Implementations may add query/mode fields around this base key, but MUST
/// NOT remove any member: a cached privileged response is invalid after any
/// principal, tenant, output-context, policy, provider, or generation change.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ReadCacheKey {
    pub pubkey: String,
    pub community_id: Uuid,
    pub context: OutputContext,
    pub policy_id: String,
    pub provider_id: String,
    pub generation: u64,
}

impl ReadScope {
    /// Effective clearance is the lower of caller and output context.
    pub fn effective_clearance(&self) -> Clearance {
        self.caller_clearance.min(self.output_clearance)
    }

    /// Validate invariants that every provider may rely on.
    pub fn validate(&self) -> Result<(), ProviderError> {
        if self.pubkey.len() != 64 || !self.pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ProviderError::InvalidRequest(
                "invalid caller pubkey".into(),
            ));
        }
        if self.policy_id.trim().is_empty() {
            return Err(ProviderError::InvalidRequest(
                "policy id is required".into(),
            ));
        }
        if let Some(id) = self.context.channel_id() {
            if !self.accessible_channel_ids.contains(&id) {
                return Err(ProviderError::InvalidRequest(
                    "output context is not accessible to caller".into(),
                ));
            }
        }
        Ok(())
    }
}

/// Supported read modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryMode {
    /// Exact current slot winners only.
    Current,
    /// Current record plus its supersession explanation.
    Why,
    /// Full append-only history visible in the scope.
    History,
}

/// Provider query request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryRequest {
    /// Relay-derived read authorization.
    pub scope: ReadScope,
    /// Read mode.
    pub mode: QueryMode,
    /// Optional case-insensitive token search.
    pub query: Option<String>,
    /// Optional exact slot filter.
    pub slot: Option<String>,
    /// Bounded result count.
    pub limit: usize,
}

/// Provider health included in every receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealth {
    Healthy,
    Degraded,
    Unavailable,
}

/// Truth state exposed by every read surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TruthStatus {
    Current,
    Proposed,
    Superseded,
    Stale,
    Disputed,
    Expired,
    Unknown,
    Unavailable,
}

/// Receipt proving what a provider asserted and at which high-water.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TruthReceipt {
    pub status: TruthStatus,
    pub slot: Option<String>,
    pub generation_high_water: u64,
    pub generation_healthy: bool,
    pub authority: Option<String>,
    pub evidence_hash: Option<String>,
    pub provider_id: String,
    pub provider_health: ProviderHealth,
    pub policy_id: String,
    pub current_winner_for_slot: Option<String>,
}

impl TruthReceipt {
    /// Whether this receipt may be rendered as green current for `record_id`.
    pub fn proves_current(&self, record_id: &str) -> bool {
        self.status == TruthStatus::Current
            && self.provider_health == ProviderHealth::Healthy
            && self.generation_healthy
            && self.slot.as_deref().is_some_and(|slot| !slot.is_empty())
            && self.current_winner_for_slot.as_deref() == Some(record_id)
    }

    /// Explicit marker used when the read plane could not reach its provider.
    pub fn unavailable(provider_id: impl Into<String>, policy_id: impl Into<String>) -> Self {
        Self {
            status: TruthStatus::Unavailable,
            slot: None,
            generation_high_water: 0,
            generation_healthy: false,
            authority: None,
            evidence_hash: None,
            provider_id: provider_id.into(),
            provider_health: ProviderHealth::Unavailable,
            policy_id: policy_id.into(),
            current_winner_for_slot: None,
        }
    }
}

/// A memory record returned by a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub slot: String,
    pub statement: String,
    pub clearance: Clearance,
    pub source_context: OutputContext,
    pub observed_at: i64,
    pub supersedes: Option<String>,
    pub authority: Option<String>,
    pub evidence_hash: Option<String>,
    pub receipt: TruthReceipt,
}

/// Successful provider query response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryResponse {
    pub results: Vec<MemoryRecord>,
    pub receipt: TruthReceipt,
    /// Distinguishes provider failure from a genuine empty result set.
    pub unavailable: bool,
    /// Deployment gate for green current rendering. Providers may prove exact
    /// winners, but the relay keeps this false until its oracle gate is accepted.
    pub current_rendering_enabled: bool,
}

/// Provider status request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusRequest {
    pub scope: ReadScope,
}

/// Provider status response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub provider_id: String,
    pub health: ProviderHealth,
    pub generation_high_water: u64,
    pub generation_healthy: bool,
    pub policy_id: String,
    pub receipt: TruthReceipt,
}

/// Typed provider failure. Store contents must never appear in these messages.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    #[error("memory provider unavailable")]
    Unavailable,
    #[error("memory provider timed out")]
    Timeout,
    #[error("invalid memory request: {0}")]
    InvalidRequest(String),
    #[error("memory provider internal error")]
    Internal,
}

/// Provider-neutral object-safe read contract.
#[async_trait]
pub trait MemoryProvider: Send + Sync {
    /// Stable implementation identifier included in cache keys and receipts.
    fn provider_id(&self) -> &str;
    /// Return health and generation for one authorized output context.
    async fn status(&self, request: StatusRequest) -> Result<ProviderStatus, ProviderError>;
    /// Query records visible to one authorized output context.
    async fn query(&self, request: QueryRequest) -> Result<QueryResponse, ProviderError>;
}

#[derive(Debug, Clone)]
struct StoredRecord {
    record: MemoryRecord,
    community_id: Uuid,
    tokens: BTreeSet<String>,
}

#[derive(Debug, Default)]
struct NaiveState {
    generation: u64,
    records: BTreeMap<String, StoredRecord>,
    winners: BTreeMap<(Uuid, OutputContext, String), String>,
}

/// Auditable in-memory LWW engine used by the upstream default integration.
#[derive(Debug, Clone, Default)]
pub struct NaiveProvider {
    state: Arc<RwLock<NaiveState>>,
}

impl NaiveProvider {
    /// Create an empty provider.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a record for tests or a deployment-owned ingestion adapter.
    ///
    /// This is intentionally not part of [`MemoryProvider`]: stage one exposes
    /// reads only. LWW compares `(observed_at, id)` and retains all losers for
    /// history. A non-empty `supersedes` target must already exist in the same
    /// community and slot.
    pub async fn insert(
        &self,
        community_id: Uuid,
        mut record: MemoryRecord,
    ) -> Result<u64, ProviderError> {
        if record.id.trim().is_empty() || record.slot.trim().is_empty() {
            return Err(ProviderError::InvalidRequest(
                "record id and slot are required".into(),
            ));
        }
        let mut state = self.state.write().await;
        if state.records.contains_key(&record.id) {
            return Err(ProviderError::InvalidRequest("duplicate record id".into()));
        }
        if let Some(target_id) = record.supersedes.as_deref() {
            let target = state.records.get(target_id).ok_or_else(|| {
                ProviderError::InvalidRequest("supersedes target does not exist".into())
            })?;
            if target.community_id != community_id || target.record.slot != record.slot {
                return Err(ProviderError::InvalidRequest(
                    "supersedes target must share community and slot".into(),
                ));
            }
        }
        record.receipt.provider_id = NAIVE_PROVIDER_ID.to_string();
        let key = (
            community_id,
            record.source_context.clone(),
            record.slot.clone(),
        );
        let wins = state
            .winners
            .get(&key)
            .and_then(|id| state.records.get(id))
            .map(|winner| {
                (record.observed_at, record.id.as_str())
                    > (winner.record.observed_at, winner.record.id.as_str())
            })
            .unwrap_or(true);
        let id = record.id.clone();
        let tokens = tokenize(&record.statement);
        state.records.insert(
            id.clone(),
            StoredRecord {
                record,
                community_id,
                tokens,
            },
        );
        if wins {
            state.winners.insert(key, id);
        }
        state.generation = state.generation.saturating_add(1);
        Ok(state.generation)
    }
}

#[async_trait]
impl MemoryProvider for NaiveProvider {
    fn provider_id(&self) -> &str {
        NAIVE_PROVIDER_ID
    }

    async fn status(&self, request: StatusRequest) -> Result<ProviderStatus, ProviderError> {
        request.scope.validate()?;
        let generation = self.state.read().await.generation;
        let receipt = TruthReceipt {
            status: TruthStatus::Unknown,
            slot: None,
            generation_high_water: generation,
            generation_healthy: true,
            authority: Some("naive_lww".into()),
            evidence_hash: None,
            provider_id: NAIVE_PROVIDER_ID.into(),
            provider_health: ProviderHealth::Healthy,
            policy_id: request.scope.policy_id.clone(),
            current_winner_for_slot: None,
        };
        Ok(ProviderStatus {
            provider_id: NAIVE_PROVIDER_ID.into(),
            health: ProviderHealth::Healthy,
            generation_high_water: generation,
            generation_healthy: true,
            policy_id: request.scope.policy_id,
            receipt,
        })
    }

    async fn query(&self, request: QueryRequest) -> Result<QueryResponse, ProviderError> {
        request.scope.validate()?;
        if request.limit == 0 || request.limit > MAX_QUERY_LIMIT {
            return Err(ProviderError::InvalidRequest(format!(
                "limit must be in 1..={MAX_QUERY_LIMIT}"
            )));
        }
        let needle = request.query.as_deref().map(tokenize);
        let effective = request.scope.effective_clearance();
        let state = self.state.read().await;
        let mut rows: Vec<&StoredRecord> = state
            .records
            .values()
            .filter(|stored| stored.community_id == request.scope.community_id)
            .filter(|stored| stored.record.clearance <= effective)
            .filter(|stored| context_visible(&request.scope, &stored.record.source_context))
            .filter(|stored| {
                request
                    .slot
                    .as_deref()
                    .is_none_or(|slot| stored.record.slot == slot)
            })
            .filter(|stored| {
                needle
                    .as_ref()
                    .is_none_or(|wanted| wanted.iter().all(|term| stored.tokens.contains(term)))
            })
            .filter(|stored| match request.mode {
                QueryMode::History => true,
                QueryMode::Current | QueryMode::Why => {
                    let key = (
                        stored.community_id,
                        stored.record.source_context.clone(),
                        stored.record.slot.clone(),
                    );
                    state.winners.get(&key) == Some(&stored.record.id)
                }
            })
            .collect();
        rows.sort_by(|a, b| {
            (b.record.observed_at, b.record.id.as_str())
                .cmp(&(a.record.observed_at, a.record.id.as_str()))
        });
        rows.truncate(request.limit);

        let mut results = Vec::with_capacity(rows.len());
        for stored in rows {
            let key = (
                stored.community_id,
                stored.record.source_context.clone(),
                stored.record.slot.clone(),
            );
            let winner = state.winners.get(&key).cloned();
            let is_current = winner.as_deref() == Some(stored.record.id.as_str());
            let mut record = stored.record.clone();
            record.receipt = TruthReceipt {
                status: if is_current {
                    TruthStatus::Current
                } else {
                    TruthStatus::Superseded
                },
                slot: Some(record.slot.clone()),
                generation_high_water: state.generation,
                generation_healthy: true,
                authority: record
                    .authority
                    .clone()
                    .or_else(|| Some("naive_lww".into())),
                evidence_hash: record.evidence_hash.clone(),
                provider_id: NAIVE_PROVIDER_ID.into(),
                provider_health: ProviderHealth::Healthy,
                policy_id: request.scope.policy_id.clone(),
                current_winner_for_slot: winner,
            };
            results.push(record);
        }
        let aggregate_current =
            results.len() == 1 && results[0].receipt.proves_current(&results[0].id);
        let status = if results.is_empty() {
            TruthStatus::Unknown
        } else if aggregate_current {
            TruthStatus::Current
        } else {
            TruthStatus::Superseded
        };
        let aggregate_slot = aggregate_current
            .then(|| results[0].slot.clone())
            .or(request.slot);
        let aggregate_winner = aggregate_current.then(|| results[0].id.clone());
        Ok(QueryResponse {
            results,
            receipt: TruthReceipt {
                status,
                slot: aggregate_slot,
                generation_high_water: state.generation,
                generation_healthy: true,
                authority: Some("naive_lww".into()),
                evidence_hash: None,
                provider_id: NAIVE_PROVIDER_ID.into(),
                provider_health: ProviderHealth::Healthy,
                policy_id: request.scope.policy_id,
                current_winner_for_slot: aggregate_winner,
            },
            unavailable: false,
            current_rendering_enabled: true,
        })
    }
}

fn context_visible(scope: &ReadScope, source: &OutputContext) -> bool {
    match source {
        OutputContext::WorkspacePublic => true,
        OutputContext::Channel { id } | OutputContext::Dm { id } => {
            scope.accessible_channel_ids.contains(id) && scope.context == *source
        }
    }
}

fn tokenize(value: &str) -> BTreeSet<String> {
    value
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|part| !part.is_empty())
        .map(str::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(context: OutputContext, channels: Vec<Uuid>) -> ReadScope {
        ReadScope {
            pubkey: "a".repeat(64),
            community_id: Uuid::from_u128(1),
            context,
            accessible_channel_ids: channels,
            caller_clearance: Clearance::Restricted,
            output_clearance: Clearance::Restricted,
            policy_id: "policy-v1".into(),
        }
    }

    fn record(
        id: &str,
        slot: &str,
        statement: &str,
        at: i64,
        context: OutputContext,
    ) -> MemoryRecord {
        MemoryRecord {
            id: id.into(),
            slot: slot.into(),
            statement: statement.into(),
            clearance: Clearance::Restricted,
            source_context: context,
            observed_at: at,
            supersedes: None,
            authority: None,
            evidence_hash: None,
            receipt: TruthReceipt::unavailable("unbound", "unbound"),
        }
    }

    #[tokio::test]
    async fn current_only_returns_exact_slot_winner() {
        let provider = NaiveProvider::new();
        let channel = Uuid::from_u128(7);
        let context = OutputContext::Channel { id: channel };
        provider
            .insert(
                Uuid::from_u128(1),
                record("old", "deploy", "old version", 1, context.clone()),
            )
            .await
            .unwrap();
        let mut newer = record("new", "deploy", "new version", 2, context.clone());
        newer.supersedes = Some("old".into());
        provider.insert(Uuid::from_u128(1), newer).await.unwrap();
        let response = provider
            .query(QueryRequest {
                scope: scope(context, vec![channel]),
                mode: QueryMode::Current,
                query: None,
                slot: Some("deploy".into()),
                limit: 10,
            })
            .await
            .unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].id, "new");
        assert!(response.results[0].receipt.proves_current("new"));
    }

    #[tokio::test]
    async fn history_retains_superseded_row_and_fts_filters() {
        let provider = NaiveProvider::new();
        let channel = Uuid::from_u128(7);
        let context = OutputContext::Channel { id: channel };
        provider
            .insert(
                Uuid::from_u128(1),
                record("old", "deploy", "relay blue", 1, context.clone()),
            )
            .await
            .unwrap();
        provider
            .insert(
                Uuid::from_u128(1),
                record("other", "owner", "Nathan", 2, context.clone()),
            )
            .await
            .unwrap();
        let response = provider
            .query(QueryRequest {
                scope: scope(context, vec![channel]),
                mode: QueryMode::History,
                query: Some("blue".into()),
                slot: None,
                limit: 10,
            })
            .await
            .unwrap();
        assert_eq!(
            response
                .results
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["old"]
        );
    }

    #[tokio::test]
    async fn restricted_data_never_crosses_output_context() {
        let provider = NaiveProvider::new();
        let private = Uuid::from_u128(7);
        let public = Uuid::from_u128(8);
        provider
            .insert(
                Uuid::from_u128(1),
                record(
                    "secret",
                    "incident",
                    "private root cause",
                    1,
                    OutputContext::Channel { id: private },
                ),
            )
            .await
            .unwrap();
        let response = provider
            .query(QueryRequest {
                scope: scope(OutputContext::Channel { id: public }, vec![private, public]),
                mode: QueryMode::History,
                query: None,
                slot: None,
                limit: 10,
            })
            .await
            .unwrap();
        assert!(response.results.is_empty());
        assert_eq!(response.receipt.status, TruthStatus::Unknown);
        assert!(!response.unavailable);
        assert!(response.current_rendering_enabled);
    }

    #[test]
    fn malformed_current_receipt_is_not_green() {
        let mut receipt = TruthReceipt::unavailable(NAIVE_PROVIDER_ID, "policy-v1");
        receipt.status = TruthStatus::Current;
        receipt.provider_health = ProviderHealth::Healthy;
        receipt.generation_healthy = true;
        receipt.slot = Some("deploy".into());
        assert!(!receipt.proves_current("winner"));
    }
}
