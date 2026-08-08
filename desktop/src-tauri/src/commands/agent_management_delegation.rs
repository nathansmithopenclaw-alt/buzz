use std::collections::BTreeMap;

use buzz_core_pkg::observer::encrypt_observer_payload;
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppState,
    managed_agents::{
        load_global_agent_config,
        storage::{atomic_write_json_restricted, managed_agents_base_dir},
        validate_delegated_env_vars, AgentManagementDelegationGrant,
    },
    util::now_iso,
};

mod result_publication;
use result_publication::{
    publish_agent_management_applying_result, publish_agent_management_result,
};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelegatedCapability {
    AgentConfig,
    AgentCreation,
    EnvVars,
    Teams,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RequestStatus {
    PendingDraft,
    Applying,
    Applied,
    Failed,
    Dismissed,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RequestLedgerEntry {
    source_pubkey: String,
    capability: DelegatedCapability,
    payload_sha256: String,
    payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    audit_channel_id: Option<String>,
    #[serde(default)]
    audit_published: bool,
    #[serde(default)]
    owner_reviewed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    audit_event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret_response_event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result_event: Option<String>,
    #[serde(default = "legacy_result_was_published")]
    result_published: bool,
    status: RequestStatus,
    created_at: String,
    updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct AgentManagementState {
    #[serde(default)]
    requests: BTreeMap<String, RequestLedgerEntry>,
}

// Before result publication was tracked, terminal paths published immediately.
// Treat legacy terminal entries as already delivered so an upgrade does not
// replay every historical failure; startup recovery explicitly resets this for
// interrupted `applying` entries.
fn legacy_result_was_published() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagementRequestDisposition {
    Apply,
    Draft,
    Rejected,
    DuplicateApplying,
    DuplicateApplied,
    DuplicateDraft,
    DuplicateFailed,
    DuplicateDismissed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterManagementRequestResult {
    pub disposition: ManagementRequestDisposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit_channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoredManagementRequest {
    pub request_id: String,
    pub source_pubkey: String,
    pub capability: DelegatedCapability,
    pub payload: Value,
    pub disposition: RestoredManagementRequestDisposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestoredManagementRequestDisposition {
    Draft,
    Failed,
}

fn state_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join("agent-management-state.json"))
}

fn load_state(app: &AppHandle) -> Result<AgentManagementState, String> {
    let path = state_path(app)?;
    if !path.exists() {
        return Ok(AgentManagementState::default());
    }
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("failed to read agent management state: {error}"))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to parse agent management state: {error}"))
}

fn save_state(app: &AppHandle, state: &AgentManagementState) -> Result<(), String> {
    let payload = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("failed to serialize agent management state: {error}"))?;
    atomic_write_json_restricted(&state_path(app)?, &payload)
}

fn payload_sha256(payload: &Value) -> Result<String, String> {
    let canonical = serde_json::to_vec(payload)
        .map_err(|error| format!("failed to serialize management request: {error}"))?;
    Ok(hex::encode(Sha256::digest(canonical)))
}

fn canonical_request_id(request_id: &str) -> Result<String, String> {
    uuid::Uuid::parse_str(request_id.trim())
        .map_err(|_| "invalid management request UUID".to_string())
        .map(|request_id| request_id.to_string())
}

fn capability_enabled(
    grant: &AgentManagementDelegationGrant,
    capability: DelegatedCapability,
) -> bool {
    match capability {
        DelegatedCapability::AgentConfig => false,
        DelegatedCapability::AgentCreation => grant.payload.capabilities.agent_creation,
        DelegatedCapability::EnvVars => grant.payload.capabilities.env_vars,
        DelegatedCapability::Teams => grant.payload.capabilities.teams,
    }
}

struct NewLedgerRequest {
    source_pubkey: String,
    capability: DelegatedCapability,
    payload_sha256: String,
    payload: Value,
    authorized: bool,
    audit_channel_id: Option<String>,
    rejection: Option<String>,
}

fn register_in_ledger(
    ledger: &mut AgentManagementState,
    request_id: String,
    request: NewLedgerRequest,
) -> Result<RegisterManagementRequestResult, String> {
    if let Some(existing) = ledger.requests.get(&request_id) {
        if existing.source_pubkey != request.source_pubkey
            || existing.capability != request.capability
            || existing.payload_sha256 != request.payload_sha256
        {
            return Err("management request UUID was reused with different content".into());
        }
        let disposition = match existing.status {
            RequestStatus::PendingDraft => ManagementRequestDisposition::DuplicateDraft,
            RequestStatus::Applying => ManagementRequestDisposition::DuplicateApplying,
            RequestStatus::Applied => ManagementRequestDisposition::DuplicateApplied,
            RequestStatus::Failed => ManagementRequestDisposition::DuplicateFailed,
            RequestStatus::Dismissed => ManagementRequestDisposition::DuplicateDismissed,
        };
        return Ok(RegisterManagementRequestResult {
            disposition,
            audit_channel_id: existing.audit_channel_id.clone(),
            result: existing.result.clone(),
        });
    }

    let now = now_iso();
    let rejection_result = request
        .rejection
        .as_ref()
        .map(|error| serde_json::json!({ "code": "invalid_env_vars", "error": error }));
    let rejected = rejection_result.is_some();
    let status = if rejected {
        RequestStatus::Failed
    } else if request.authorized {
        RequestStatus::Applying
    } else {
        RequestStatus::PendingDraft
    };
    ledger.requests.insert(
        request_id,
        RequestLedgerEntry {
            source_pubkey: request.source_pubkey,
            capability: request.capability,
            payload_sha256: request.payload_sha256,
            payload: request.payload,
            audit_channel_id: request.audit_channel_id.clone(),
            audit_published: false,
            owner_reviewed: false,
            audit_event: None,
            secret_response_event: None,
            result_event: None,
            result_published: false,
            status,
            created_at: now.clone(),
            updated_at: now,
            result: rejection_result.clone(),
        },
    );
    Ok(RegisterManagementRequestResult {
        disposition: if rejected {
            ManagementRequestDisposition::Rejected
        } else if request.authorized {
            ManagementRequestDisposition::Apply
        } else {
            ManagementRequestDisposition::Draft
        },
        audit_channel_id: request.audit_channel_id,
        result: rejection_result,
    })
}

fn request_env_vars(payload: &Value) -> Result<Option<BTreeMap<String, String>>, String> {
    let Some(value) = payload
        .get("request")
        .and_then(|request| request.get("envVars"))
    else {
        return Ok(None);
    };
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|_| "envVars must be an object with string values".to_string())
}

fn env_fingerprint_summary(payload: &Value) -> Option<String> {
    let env_vars = request_env_vars(payload).ok().flatten()?;
    if env_vars.is_empty() {
        return None;
    }
    Some(
        env_vars
            .into_iter()
            .map(|(key, value)| format!("{key}=sha256:{}", hex::encode(Sha256::digest(value))))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

/// Durably classify a request before any side effect.
///
/// New authorized applies are written as `applying` before the caller mutates
/// agent/team state. A replay after a crash therefore fails closed instead of
/// re-running a non-idempotent create. Requests without a live matching grant
/// are durably queued for owner review.
pub fn register_agent_management_request(
    source_pubkey: String,
    request_id: String,
    capability: DelegatedCapability,
    intent: String,
    payload: Value,
    app: AppHandle,
) -> Result<RegisterManagementRequestResult, String> {
    let source = PublicKey::parse(source_pubkey.trim())
        .map_err(|error| format!("invalid management requester pubkey: {error}"))?
        .to_hex();
    let request_id = canonical_request_id(&request_id)?;
    if intent != "apply" && intent != "draft" {
        return Err("management intent must be apply or draft".into());
    }
    let digest = payload_sha256(&payload)?;
    let state_handle = app.state::<AppState>();
    let owner = state_handle.signing_keys()?.public_key();
    let _guard = state_handle
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let grant = load_global_agent_config(&app)
        .ok()
        .and_then(|config| config.agent_management_delegation)
        .filter(|grant| grant.validate(&owner).is_ok());
    let audit_channel_id = grant
        .as_ref()
        .map(|grant| grant.payload.audit_channel_id.clone());
    let env_vars = request_env_vars(&payload)?;
    let authorized = intent == "apply"
        && grant.as_ref().is_some_and(|grant| {
            grant.payload.grantee_pubkey == source
                && capability_enabled(grant, capability)
                && (env_vars.is_none() || grant.payload.capabilities.env_vars)
        });
    let rejection = if authorized {
        env_vars
            .as_ref()
            .and_then(|values| validate_delegated_env_vars(values).err())
    } else {
        None
    };

    let mut ledger = load_state(&app)?;
    let result = register_in_ledger(
        &mut ledger,
        request_id,
        NewLedgerRequest {
            source_pubkey: source,
            capability,
            payload_sha256: digest,
            payload,
            authorized,
            audit_channel_id,
            rejection,
        },
    )?;
    save_state(&app, &ledger)?;
    Ok(result)
}

pub fn complete_agent_management_request(
    request_id: String,
    succeeded: bool,
    result: Value,
    app: AppHandle,
) -> Result<(), String> {
    let request_id = canonical_request_id(&request_id)?;
    if value_contains_secret(&result) {
        return Err("management ledger results must not contain secrets".into());
    }
    let state_handle = app.state::<AppState>();
    let _guard = state_handle
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut ledger = load_state(&app)?;
    let entry = ledger
        .requests
        .get_mut(&request_id)
        .ok_or("management request is not registered")?;
    if entry.status != RequestStatus::Applying {
        return Err("management request is not in applying state".into());
    }
    entry.status = if succeeded {
        RequestStatus::Applied
    } else {
        RequestStatus::Failed
    };
    entry.updated_at = now_iso();
    entry.result = Some(result);
    save_state(&app, &ledger)
}

fn begin_owner_reviewed_in_ledger(
    ledger: &mut AgentManagementState,
    request_id: &str,
) -> Result<(), String> {
    let entry = ledger
        .requests
        .get_mut(request_id)
        .ok_or("management request is not registered")?;
    if entry.status != RequestStatus::PendingDraft {
        return Err("management request is not pending owner review".into());
    }
    entry.status = RequestStatus::Applying;
    entry.owner_reviewed = true;
    entry.updated_at = now_iso();
    Ok(())
}

/// Move a durable draft into the apply phase after an explicit owner action.
///
/// This is intentionally separate from grant authorization: approving one
/// queued request does not create or widen a standing delegation grant.
pub fn begin_owner_reviewed_agent_management_request(
    request_id: String,
    app: AppHandle,
) -> Result<(), String> {
    let request_id = canonical_request_id(&request_id)?;
    let state_handle = app.state::<AppState>();
    let _guard = state_handle
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut ledger = load_state(&app)?;
    begin_owner_reviewed_in_ledger(&mut ledger, &request_id)?;
    save_state(&app, &ledger)
}

fn value_contains_secret(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(key, value)| {
            let key = key.to_ascii_lowercase();
            key.contains("nsec")
                || key.contains("private_key")
                || key.contains("privatekey")
                || value_contains_secret(value)
        }),
        Value::Array(values) => values.iter().any(value_contains_secret),
        _ => false,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentCreateSecretResponse<'a> {
    #[serde(rename = "type")]
    response_type: &'static str,
    request_id: &'a str,
    pubkey: &'a str,
    nsec: &'a str,
}

/// Publish the one-time create secret response as owner-signed NIP-44
/// ciphertext addressed only to the grantee. The public audit receipt and the
/// durable request ledger never receives the plaintext tuple. The exact signed
/// ciphertext event is retained so retries cannot emit multiple secret events.
pub async fn publish_agent_create_secret_response(
    request_id: String,
    source_pubkey: String,
    agent_pubkey: String,
    private_key_nsec: String,
    app: AppHandle,
) -> Result<(), String> {
    let request_id = canonical_request_id(&request_id)?;
    let source = PublicKey::parse(source_pubkey.trim())
        .map_err(|error| format!("invalid management requester pubkey: {error}"))?;
    let agent = PublicKey::parse(agent_pubkey.trim())
        .map_err(|error| format!("invalid created agent pubkey: {error}"))?;
    let agent_keys = Keys::parse(private_key_nsec.trim())
        .map_err(|error| format!("invalid created agent secret: {error}"))?;
    if agent_keys.public_key() != agent {
        return Err("created agent secret does not match its pubkey".into());
    }
    let source_hex = source.to_hex();
    let agent_hex = agent.to_hex();
    let owner_keys = app.state::<AppState>().signing_keys()?;
    let secret_event = {
        let state_handle = app.state::<AppState>();
        let _guard = state_handle
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut ledger = load_state(&app)?;
        let entry = ledger
            .requests
            .get_mut(&request_id)
            .ok_or("management request is not registered")?;
        let recorded_pubkey = entry
            .result
            .as_ref()
            .and_then(|value| value.get("pubkey"))
            .and_then(Value::as_str);
        if entry.status != RequestStatus::Applied
            || entry.source_pubkey != source_hex
            || entry.capability != DelegatedCapability::AgentCreation
            || recorded_pubkey != Some(agent_hex.as_str())
        {
            return Err("create secret response does not match the applied request".into());
        }
        if let Some(raw) = &entry.secret_response_event {
            Event::from_json(raw)
                .map_err(|error| format!("stored create-secret response is invalid: {error}"))?
        } else {
            let response = AgentCreateSecretResponse {
                response_type: "agent_create_secret_response",
                request_id: &request_id,
                pubkey: &agent_hex,
                nsec: private_key_nsec.trim(),
            };
            let encrypted = encrypt_observer_payload(&owner_keys, &source, &response)
                .map_err(|error| format!("failed to encrypt create result: {error}"))?;
            let d_tag = format!("buzz-agent-create-result:{request_id}");
            let d = Tag::parse(["d", d_tag.as_str()])
                .map_err(|error| format!("failed to build create-result d tag: {error}"))?;
            let p = Tag::parse(["p", source_hex.as_str()])
                .map_err(|error| format!("failed to build create-result p tag: {error}"))?;
            let event = EventBuilder::new(Kind::Custom(30_078), encrypted)
                .tags([d, p])
                .sign_with_keys(&owner_keys)
                .map_err(|error| format!("failed to sign create result: {error}"))?;
            entry.secret_response_event = Some(event.as_json());
            entry.updated_at = now_iso();
            save_state(&app, &ledger)?;
            event
        }
    };
    crate::relay::submit_signed_event_with_keys(
        &secret_event,
        &app.state::<AppState>(),
        &owner_keys,
        None,
    )
    .await
    .map(|_| ())
}

pub async fn publish_agent_management_audit_receipt(
    request_id: String,
    source_pubkey: String,
    requester_name: String,
    capability: DelegatedCapability,
    target: String,
    succeeded: bool,
    app: AppHandle,
) -> Result<(), String> {
    let request_id = canonical_request_id(&request_id)?;
    let source = PublicKey::parse(source_pubkey.trim())
        .map_err(|error| format!("invalid management requester pubkey: {error}"))?;
    let owner_keys = app.state::<AppState>().signing_keys()?;
    let source_hex = source.to_hex();
    let (channel, stored_audit_event, owner_reviewed, env_summary) = {
        let state_handle = app.state::<AppState>();
        let _guard = state_handle
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let ledger = load_state(&app)?;
        let entry = ledger
            .requests
            .get(&request_id)
            .ok_or("management request is not registered")?;
        if entry.source_pubkey != source_hex || entry.capability != capability {
            return Err("audit receipt does not match the registered request".into());
        }
        let expected_status = if succeeded {
            RequestStatus::Applied
        } else {
            RequestStatus::Failed
        };
        if entry.status != expected_status {
            return Err("audit receipt does not match the terminal request status".into());
        }
        if entry.audit_published {
            return Ok(());
        }
        (
            entry
                .audit_channel_id
                .clone()
                .ok_or("management request has no authorization audit channel")?,
            entry.audit_event.clone(),
            entry.owner_reviewed,
            env_fingerprint_summary(&entry.payload),
        )
    };
    let safe_name: String = requester_name
        .chars()
        .filter(|character| !character.is_control() && *character != '@')
        .take(120)
        .collect();
    let safe_target: String = target
        .chars()
        .filter(|character| !character.is_control())
        .take(240)
        .collect();
    let capability_label = match capability {
        DelegatedCapability::AgentConfig => "agent config",
        DelegatedCapability::AgentCreation => "agent creation",
        DelegatedCapability::EnvVars => "agent env vars",
        DelegatedCapability::Teams => "teams",
    };
    let outcome = if succeeded { "applied" } else { "failed" };
    let authority = if owner_reviewed {
        "owner-reviewed"
    } else {
        "delegated"
    };
    let mut content = format!(
        "@{safe_name} {authority} {capability_label} {outcome}: {safe_target}\n\nRequest: `{}`",
        request_id
    );
    if let Some(summary) = env_summary {
        content.push_str("\nEnv fingerprints: ");
        content.push_str(&summary);
    }
    let h = Tag::parse(["h", channel.as_str()])
        .map_err(|error| format!("failed to build audit channel tag: {error}"))?;
    let p = Tag::parse(["p", source_hex.as_str()])
        .map_err(|error| format!("failed to build audit mention tag: {error}"))?;
    let mut audit_event = match stored_audit_event {
        Some(raw) => Event::from_json(raw)
            .map_err(|error| format!("stored audit receipt is invalid: {error}"))?,
        None => EventBuilder::new(Kind::Custom(9), content)
            .tags([h, p])
            .sign_with_keys(&owner_keys)
            .map_err(|error| format!("failed to sign audit receipt: {error}"))?,
    };
    {
        let state_handle = app.state::<AppState>();
        let _guard = state_handle
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut ledger = load_state(&app)?;
        let entry = ledger
            .requests
            .get_mut(&request_id)
            .ok_or("management request disappeared before audit publication")?;
        if let Some(raw) = &entry.audit_event {
            audit_event = Event::from_json(raw)
                .map_err(|error| format!("stored audit receipt is invalid: {error}"))?;
        } else {
            entry.audit_event = Some(audit_event.as_json());
            entry.updated_at = now_iso();
            save_state(&app, &ledger)?;
        }
    }
    crate::relay::submit_signed_event_with_keys(
        &audit_event,
        &app.state::<AppState>(),
        &owner_keys,
        None,
    )
    .await?;
    let state_handle = app.state::<AppState>();
    let _guard = state_handle
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut ledger = load_state(&app)?;
    let entry = ledger
        .requests
        .get_mut(&request_id)
        .ok_or("management request disappeared after audit publication")?;
    entry.audit_published = true;
    entry.updated_at = now_iso();
    save_state(&app, &ledger)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagementDraftResult<'a> {
    #[serde(rename = "type")]
    result_type: &'static str,
    request_id: &'a str,
    succeeded: bool,
    result: Value,
}

pub async fn publish_agent_management_draft_result(
    request_id: String,
    source_pubkey: String,
    app: AppHandle,
) -> Result<(), String> {
    let request_id = canonical_request_id(&request_id)?;
    let source = PublicKey::parse(source_pubkey.trim())
        .map_err(|error| format!("invalid management requester pubkey: {error}"))?;
    let source_hex = source.to_hex();
    {
        let state_handle = app.state::<AppState>();
        let _guard = state_handle
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let ledger = load_state(&app)?;
        let entry = ledger
            .requests
            .get(&request_id)
            .ok_or("management request is not registered")?;
        if entry.status != RequestStatus::PendingDraft || entry.source_pubkey != source_hex {
            return Err("management request is not pending owner review".into());
        }
    }
    let owner_keys = app.state::<AppState>().signing_keys()?;
    let content = encrypt_observer_payload(
        &owner_keys,
        &source,
        &ManagementDraftResult {
            result_type: "agent_management_apply_result",
            request_id: &request_id,
            succeeded: false,
            result: serde_json::json!({"queuedForReview": true}),
        },
    )
    .map_err(|error| format!("failed to encrypt draft result: {error}"))?;
    let d_tag = format!("buzz-agent-management-result:{request_id}");
    let d = Tag::parse(["d", d_tag.as_str()])
        .map_err(|error| format!("failed to build draft-result d tag: {error}"))?;
    let p = Tag::parse(["p", source_hex.as_str()])
        .map_err(|error| format!("failed to build draft-result p tag: {error}"))?;
    crate::relay::submit_event_with_keys(
        EventBuilder::new(Kind::Custom(30_078), content).tags([d, p]),
        &app.state::<AppState>(),
        &owner_keys,
        None,
    )
    .await
    .map(|_| ())
}

fn restore_requests(
    ledger: &mut AgentManagementState,
    startup_cutoff: chrono::DateTime<chrono::FixedOffset>,
) -> (Vec<RestoredManagementRequest>, bool) {
    let interrupted_result = serde_json::json!({
        "code": "interrupted_during_apply",
        "error": "Desktop restarted while this management request was applying; retry with a new request id"
    });
    let mut changed = false;
    for entry in ledger.requests.values_mut() {
        let predates_startup = chrono::DateTime::parse_from_rfc3339(&entry.updated_at)
            .map(|updated_at| updated_at < startup_cutoff)
            .unwrap_or(true);
        if entry.status == RequestStatus::Applying && predates_startup {
            entry.status = RequestStatus::Failed;
            entry.updated_at = now_iso();
            entry.result = Some(interrupted_result.clone());
            entry.result_event = None;
            entry.result_published = false;
            changed = true;
        }
    }

    let mut restored = ledger
        .requests
        .iter()
        .filter(|(_, entry)| {
            entry.status == RequestStatus::PendingDraft
                || (entry.status == RequestStatus::Failed && !entry.result_published)
        })
        .map(|(request_id, entry)| {
            (
                entry.created_at.clone(),
                RestoredManagementRequest {
                    request_id: request_id.clone(),
                    source_pubkey: entry.source_pubkey.clone(),
                    capability: entry.capability,
                    payload: entry.payload.clone(),
                    disposition: if entry.status == RequestStatus::PendingDraft {
                        RestoredManagementRequestDisposition::Draft
                    } else {
                        RestoredManagementRequestDisposition::Failed
                    },
                    result: entry.result.clone(),
                },
            )
        })
        .collect::<Vec<_>>();
    restored.sort_by(|(left_time, left), (right_time, right)| {
        left_time
            .cmp(right_time)
            .then_with(|| left.request_id.cmp(&right.request_id))
    });
    (
        restored.into_iter().map(|(_, request)| request).collect(),
        changed,
    )
}

pub fn restore_agent_management_requests(
    startup_cutoff: String,
    app: AppHandle,
) -> Result<Vec<RestoredManagementRequest>, String> {
    let startup_cutoff = chrono::DateTime::parse_from_rfc3339(startup_cutoff.trim())
        .map_err(|_| "invalid agent-management startup cutoff".to_string())?;
    let state_handle = app.state::<AppState>();
    let _guard = state_handle
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut ledger = load_state(&app)?;
    let (restored, changed) = restore_requests(&mut ledger, startup_cutoff);
    if changed {
        save_state(&app, &ledger)?;
    }
    Ok(restored)
}

pub fn dismiss_agent_management_request(request_id: String, app: AppHandle) -> Result<(), String> {
    let request_id = canonical_request_id(&request_id)?;
    let state_handle = app.state::<AppState>();
    let _guard = state_handle
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut ledger = load_state(&app)?;
    let entry = ledger
        .requests
        .get_mut(&request_id)
        .ok_or("management request is not registered")?;
    if entry.status != RequestStatus::PendingDraft {
        return Err("management request is not pending owner review".into());
    }
    entry.status = RequestStatus::Dismissed;
    entry.updated_at = now_iso();
    save_state(&app, &ledger)
}

#[derive(Deserialize)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AgentManagementLedgerCommand {
    Register {
        source_pubkey: String,
        request_id: String,
        capability: DelegatedCapability,
        intent: String,
        payload: Value,
    },
    Complete {
        request_id: String,
        succeeded: bool,
        result: Value,
    },
    BeginOwnerReview {
        request_id: String,
    },
    Restore {
        startup_cutoff: String,
    },
    Dismiss {
        request_id: String,
    },
}

/// Typed single IPC boundary for durable request-ledger operations.
#[tauri::command]
pub fn agent_management_ledger(
    command: AgentManagementLedgerCommand,
    app: AppHandle,
) -> Result<Value, String> {
    match command {
        AgentManagementLedgerCommand::Register {
            source_pubkey,
            request_id,
            capability,
            intent,
            payload,
        } => serde_json::to_value(register_agent_management_request(
            source_pubkey,
            request_id,
            capability,
            intent,
            payload,
            app,
        )?)
        .map_err(|error| error.to_string()),
        AgentManagementLedgerCommand::Complete {
            request_id,
            succeeded,
            result,
        } => {
            complete_agent_management_request(request_id, succeeded, result, app)?;
            Ok(Value::Null)
        }
        AgentManagementLedgerCommand::BeginOwnerReview { request_id } => {
            begin_owner_reviewed_agent_management_request(request_id, app)?;
            Ok(Value::Null)
        }
        AgentManagementLedgerCommand::Restore { startup_cutoff } => {
            serde_json::to_value(restore_agent_management_requests(startup_cutoff, app)?)
                .map_err(|error| error.to_string())
        }
        AgentManagementLedgerCommand::Dismiss { request_id } => {
            dismiss_agent_management_request(request_id, app)?;
            Ok(Value::Null)
        }
    }
}

#[derive(Deserialize)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AgentManagementPublishCommand {
    CreateSecret {
        request_id: String,
        source_pubkey: String,
        agent_pubkey: String,
        private_key_nsec: String,
    },
    Audit {
        request_id: String,
        source_pubkey: String,
        requester_name: String,
        capability: DelegatedCapability,
        target: String,
        succeeded: bool,
    },
    Draft {
        request_id: String,
        source_pubkey: String,
    },
    Applying {
        request_id: String,
        source_pubkey: String,
    },
    Result {
        request_id: String,
        source_pubkey: String,
        result: Value,
    },
}

/// Typed single IPC boundary for encrypted responses and public audit receipts.
#[tauri::command]
pub async fn publish_agent_management(
    command: AgentManagementPublishCommand,
    app: AppHandle,
) -> Result<(), String> {
    match command {
        AgentManagementPublishCommand::CreateSecret {
            request_id,
            source_pubkey,
            agent_pubkey,
            private_key_nsec,
        } => {
            publish_agent_create_secret_response(
                request_id,
                source_pubkey,
                agent_pubkey,
                private_key_nsec,
                app,
            )
            .await
        }
        AgentManagementPublishCommand::Audit {
            request_id,
            source_pubkey,
            requester_name,
            capability,
            target,
            succeeded,
        } => {
            publish_agent_management_audit_receipt(
                request_id,
                source_pubkey,
                requester_name,
                capability,
                target,
                succeeded,
                app,
            )
            .await
        }
        AgentManagementPublishCommand::Draft {
            request_id,
            source_pubkey,
        } => publish_agent_management_draft_result(request_id, source_pubkey, app).await,
        AgentManagementPublishCommand::Applying {
            request_id,
            source_pubkey,
        } => publish_agent_management_applying_result(request_id, source_pubkey, app).await,
        AgentManagementPublishCommand::Result {
            request_id,
            source_pubkey,
            result,
        } => publish_agent_management_result(request_id, source_pubkey, result, app).await,
    }
}

#[cfg(test)]
#[path = "agent_management_delegation_tests.rs"]
mod tests;
