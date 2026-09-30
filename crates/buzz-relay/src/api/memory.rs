//! Authenticated Buzz Memory read plane.
//!
//! This surface deliberately mirrors `/query`: Host binds the community,
//! NIP-98 binds the caller and payload, relay membership is enforced, and the
//! relay derives the complete accessible-channel set. Provider failures are
//! converted to explicit unavailable responses rather than empty success.

use std::sync::Arc;

use axum::{
    extract::{Extension, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use buzz_memory::{
    Clearance, OutputContext, ProviderError, QueryMode, QueryRequest, QueryResponse, ReadScope,
    StatusRequest, TruthReceipt,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{nip_fi_http::admit_nip_fi_http_on_state, state::AppState};
use buzz_auth::NipFiMode;

use super::{api_error, bridge};

const MAX_BODY_BYTES: usize = 64 * 1024;
const DEFAULT_LIMIT: usize = 20;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RequestedContext {
    Channel { id: Uuid },
    Dm { id: Uuid },
    WorkspacePublic,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryBody {
    context: RequestedContext,
    mode: QueryMode,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    slot: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusBody {
    context: RequestedContext,
}

fn default_limit() -> usize {
    DEFAULT_LIMIT
}

/// Query the configured memory provider under an explicit output context.
pub async fn query(
    State(state): State<Arc<AppState>>,
    Extension(public_scheme): Extension<crate::proxy::PublicScheme>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, Response> {
    ensure_enabled(&state).map_err(IntoResponse::into_response)?;
    if body.len() > MAX_BODY_BYTES {
        return Err(
            api_error(StatusCode::PAYLOAD_TOO_LARGE, "memory request too large").into_response(),
        );
    }
    let parsed: QueryBody = serde_json::from_slice(&body)
        .map_err(|error| {
            api_error(
                StatusCode::BAD_REQUEST,
                &format!("invalid memory query: {error}"),
            )
        })
        .map_err(IntoResponse::into_response)?;
    let (scope, event_id) = authenticate_scope(
        &state,
        public_scheme,
        &headers,
        &body,
        "/memory/v1/query",
        parsed.context,
    )
    .await?;
    bridge::check_nip98_replay(&state, &scope.0, event_id)
        .await
        .map_err(IntoResponse::into_response)?;
    let request = QueryRequest {
        scope: scope.1,
        mode: parsed.mode,
        query: parsed.query,
        slot: parsed.slot,
        limit: parsed.limit,
    };
    let provider_id = state.memory_provider.provider_id().to_string();
    let policy_id = request.scope.policy_id.clone();
    let result = tokio::time::timeout(
        state.config.memory.provider_timeout,
        state.memory_provider.query(request),
    )
    .await;
    let response = match result {
        Ok(Ok(mut response)) => {
            enforce_truth_receipts(&response, &provider_id, &policy_id)
                .map_err(IntoResponse::into_response)?;
            apply_current_rendering_gate(
                &mut response,
                state.config.memory.current_rendering_enabled,
            );
            response
        }
        Ok(Err(ProviderError::InvalidRequest(message))) => {
            return Err(api_error(StatusCode::BAD_REQUEST, &message).into_response());
        }
        Ok(Err(_)) | Err(_) => unavailable_query(provider_id, policy_id),
    };
    serde_json::to_value(response).map(Json).map_err(|error| {
        super::internal_error(&format!("memory response serialization: {error}")).into_response()
    })
}

/// Return provider health and generation under an explicit output context.
pub async fn status(
    State(state): State<Arc<AppState>>,
    Extension(public_scheme): Extension<crate::proxy::PublicScheme>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, Response> {
    ensure_enabled(&state).map_err(IntoResponse::into_response)?;
    if body.len() > MAX_BODY_BYTES {
        return Err(
            api_error(StatusCode::PAYLOAD_TOO_LARGE, "memory request too large").into_response(),
        );
    }
    let parsed: StatusBody = serde_json::from_slice(&body)
        .map_err(|error| {
            api_error(
                StatusCode::BAD_REQUEST,
                &format!("invalid memory status: {error}"),
            )
        })
        .map_err(IntoResponse::into_response)?;
    let (scope, event_id) = authenticate_scope(
        &state,
        public_scheme,
        &headers,
        &body,
        "/memory/v1/status",
        parsed.context,
    )
    .await?;
    bridge::check_nip98_replay(&state, &scope.0, event_id)
        .await
        .map_err(IntoResponse::into_response)?;
    let provider_id = state.memory_provider.provider_id().to_string();
    let policy_id = scope.1.policy_id.clone();
    let result = tokio::time::timeout(
        state.config.memory.provider_timeout,
        state
            .memory_provider
            .status(StatusRequest { scope: scope.1 }),
    )
    .await;
    match result {
        Ok(Ok(response)) => {
            let mut value = serde_json::to_value(response)
                .map_err(|error| {
                    super::internal_error(&format!("memory status serialization: {error}"))
                })
                .map_err(IntoResponse::into_response)?;
            value["current_rendering_enabled"] =
                Value::Bool(state.config.memory.current_rendering_enabled);
            Ok(Json(value))
        }
        Ok(Err(ProviderError::InvalidRequest(message))) => {
            Err(api_error(StatusCode::BAD_REQUEST, &message).into_response())
        }
        Ok(Err(_)) | Err(_) => Ok(Json(json!({
            "provider_id": provider_id,
            "health": "unavailable",
            "generation_high_water": 0,
            "generation_healthy": false,
            "policy_id": policy_id,
            "receipt": TruthReceipt::unavailable(
                state.memory_provider.provider_id(),
                &state.config.memory.policy_id,
            ),
            "unavailable": true,
            "current_rendering_enabled": state.config.memory.current_rendering_enabled,
        }))),
    }
}

fn ensure_enabled(state: &AppState) -> Result<(), (StatusCode, Json<Value>)> {
    if state.config.memory.enabled {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::NOT_FOUND,
            "memory experiment is disabled",
        ))
    }
}

async fn authenticate_scope(
    state: &Arc<AppState>,
    public_scheme: crate::proxy::PublicScheme,
    headers: &HeaderMap,
    body: &[u8],
    path: &str,
    requested: RequestedContext,
) -> Result<((buzz_core::TenantContext, ReadScope), [u8; 32]), Response> {
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| {
            api_error(
                StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
        })
        .map_err(IntoResponse::into_response)?;
    let url = bridge::nip98_expected_url(public_scheme, &tenant, path);
    let nip_fi_active = !matches!(state.config.nip_fi.mode, NipFiMode::Off);
    let admission = admit_nip_fi_http_on_state(
        state,
        headers,
        bridge::make_nip98_closure_for_admission(
            headers.clone(),
            "POST",
            url,
            Some(body.to_vec()),
            state.config.require_auth_token || nip_fi_active,
            true,
        ),
    )?;
    let pubkey = *admission.proven_pubkey();
    let (event_id, signed_created_at) = admission.into_extra();
    bridge::enforce_http_admission(state, &tenant, &pubkey)
        .await
        .map_err(IntoResponse::into_response)?;
    let pubkey_bytes = pubkey.to_bytes().to_vec();
    let auth_tag = headers
        .get("x-auth-tag")
        .and_then(|value| value.to_str().ok());
    super::relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        &pubkey_bytes,
        auth_tag,
        signed_created_at,
    )
    .await
    .map_err(IntoResponse::into_response)?;
    let accessible = state
        .get_accessible_channel_ids_cached(tenant.community(), &pubkey_bytes)
        .await
        .map_err(|error| super::internal_error(&format!("channel access lookup: {error}")))
        .map_err(IntoResponse::into_response)?;

    let (context, output_clearance, caller_clearance) = match requested {
        RequestedContext::WorkspacePublic => (
            OutputContext::WorkspacePublic,
            Clearance::WorkspacePublic,
            Clearance::Restricted,
        ),
        RequestedContext::Channel { id } | RequestedContext::Dm { id } => {
            if !accessible.contains(&id) {
                return Err(
                    api_error(StatusCode::FORBIDDEN, "memory context is not accessible")
                        .into_response(),
                );
            }
            let channel = state
                .db
                .get_channel(tenant.community(), id)
                .await
                .map_err(|_| api_error(StatusCode::FORBIDDEN, "memory context is not accessible"))
                .map_err(IntoResponse::into_response)?;
            let requested_dm = matches!(requested, RequestedContext::Dm { .. });
            if (channel.channel_type == "dm") != requested_dm {
                return Err(api_error(
                    StatusCode::BAD_REQUEST,
                    "memory context kind does not match channel",
                )
                .into_response());
            }
            let role = state
                .db
                .get_member_role(tenant.community(), id, &pubkey_bytes)
                .await
                .map_err(|error| super::internal_error(&format!("channel role lookup: {error}")))
                .map_err(IntoResponse::into_response)?;
            let caller = if role.as_deref() == Some("guest") {
                Clearance::WorkspacePublic
            } else {
                Clearance::Restricted
            };
            let context = if requested_dm {
                OutputContext::Dm { id }
            } else {
                OutputContext::Channel { id }
            };
            (context, Clearance::Restricted, caller)
        }
    };

    let scope = ReadScope {
        pubkey: pubkey.to_hex(),
        community_id: *tenant.community().as_uuid(),
        context,
        accessible_channel_ids: accessible,
        caller_clearance,
        output_clearance,
        policy_id: state.config.memory.policy_id.clone(),
    };
    scope
        .validate()
        .map_err(|error| api_error(StatusCode::BAD_REQUEST, &error.to_string()))
        .map_err(IntoResponse::into_response)?;
    Ok(((tenant, scope), event_id))
}

fn apply_current_rendering_gate(response: &mut QueryResponse, enabled: bool) {
    response.current_rendering_enabled = enabled;
    if enabled {
        return;
    }
    if response.receipt.status == buzz_memory::TruthStatus::Current {
        response.receipt.status = buzz_memory::TruthStatus::Stale;
    }
    for record in &mut response.results {
        if record.receipt.status == buzz_memory::TruthStatus::Current {
            record.receipt.status = buzz_memory::TruthStatus::Stale;
        }
    }
}

fn enforce_truth_receipts(
    response: &QueryResponse,
    provider_id: &str,
    policy_id: &str,
) -> Result<(), (StatusCode, Json<Value>)> {
    let receipt_matches_scope = |receipt: &TruthReceipt| {
        receipt.provider_id == provider_id && receipt.policy_id == policy_id
    };
    if !receipt_matches_scope(&response.receipt)
        || response
            .results
            .iter()
            .any(|record| !receipt_matches_scope(&record.receipt))
    {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "memory provider returned a receipt for the wrong provider or policy",
        ));
    }
    for record in &response.results {
        if record.receipt.status == buzz_memory::TruthStatus::Current
            && !record.receipt.proves_current(&record.id)
        {
            return Err(api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "memory provider returned an invalid current receipt",
            ));
        }
    }
    if response.receipt.status == buzz_memory::TruthStatus::Current {
        let Some(record) = response
            .results
            .first()
            .filter(|_| response.results.len() == 1)
        else {
            return Err(api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "memory provider returned an invalid aggregate current receipt",
            ));
        };
        if !response.receipt.proves_current(&record.id) {
            return Err(api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "memory provider returned an invalid aggregate current receipt",
            ));
        }
    }
    Ok(())
}

fn unavailable_query(provider_id: String, policy_id: String) -> QueryResponse {
    QueryResponse {
        results: Vec::new(),
        receipt: TruthReceipt::unavailable(provider_id, policy_id),
        unavailable: true,
        current_rendering_enabled: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_is_mandatory_and_unknown_fields_fail_closed() {
        assert!(serde_json::from_value::<QueryBody>(json!({
            "mode": "current",
            "limit": 10
        }))
        .is_err());
        assert!(serde_json::from_value::<StatusBody>(json!({
            "context": { "kind": "workspace_public" },
            "community_id": Uuid::new_v4()
        }))
        .is_err());
    }

    #[test]
    fn provider_failure_is_not_an_ordinary_empty_result() {
        let response = unavailable_query("test-provider".into(), "policy-v1".into());
        assert!(response.results.is_empty());
        assert!(response.unavailable);
        assert!(!response.current_rendering_enabled);
        assert_eq!(
            response.receipt.status,
            buzz_memory::TruthStatus::Unavailable
        );
    }

    #[test]
    fn k1_gate_prevents_green_current_rendering() {
        let mut response = QueryResponse {
            results: Vec::new(),
            receipt: TruthReceipt {
                status: buzz_memory::TruthStatus::Current,
                slot: Some("slot".into()),
                generation_high_water: 1,
                generation_healthy: true,
                authority: None,
                evidence_hash: None,
                provider_id: "provider".into(),
                provider_health: buzz_memory::ProviderHealth::Healthy,
                policy_id: "policy".into(),
                current_winner_for_slot: Some("winner".into()),
            },
            unavailable: false,
            current_rendering_enabled: true,
        };
        apply_current_rendering_gate(&mut response, false);
        assert_eq!(response.receipt.status, buzz_memory::TruthStatus::Stale);
        assert!(!response.current_rendering_enabled);
    }

    #[test]
    fn receipt_validation_precedes_the_k1_rendering_gate() {
        let response = QueryResponse {
            results: vec![buzz_memory::MemoryRecord {
                id: "winner".into(),
                slot: "slot".into(),
                statement: "statement".into(),
                clearance: Clearance::WorkspacePublic,
                source_context: OutputContext::WorkspacePublic,
                observed_at: 1,
                supersedes: None,
                authority: None,
                evidence_hash: None,
                receipt: TruthReceipt {
                    status: buzz_memory::TruthStatus::Current,
                    slot: Some("slot".into()),
                    generation_high_water: 1,
                    generation_healthy: true,
                    authority: None,
                    evidence_hash: None,
                    provider_id: "provider".into(),
                    provider_health: buzz_memory::ProviderHealth::Healthy,
                    policy_id: "policy".into(),
                    current_winner_for_slot: Some("different-record".into()),
                },
            }],
            receipt: TruthReceipt {
                status: buzz_memory::TruthStatus::Current,
                slot: Some("slot".into()),
                generation_high_water: 1,
                generation_healthy: true,
                authority: None,
                evidence_hash: None,
                provider_id: "provider".into(),
                provider_health: buzz_memory::ProviderHealth::Healthy,
                policy_id: "policy".into(),
                current_winner_for_slot: Some("winner".into()),
            },
            unavailable: false,
            current_rendering_enabled: true,
        };
        assert!(enforce_truth_receipts(&response, "provider", "policy").is_err());
    }
}
