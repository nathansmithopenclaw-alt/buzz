//! Called under the same store lock as grant revocation and the persona write.
use super::*;
use crate::managed_agents::{
    global_config::{canonical_relay_scope, AgentManagementDelegationPayload},
    load_managed_agents, load_personas, AgentDefinition, GlobalAgentConfig, ManagedAgentRecord,
    UpdatePersonaRequest,
};

pub(super) struct EditAuthority<'a> {
    pub config: &'a GlobalAgentConfig,
    pub owner: &'a PublicKey,
    pub relay: &'a str,
}

pub(super) fn authorize_payload(
    authority: &EditAuthority<'_>,
    source: &str,
    payload: &Value,
    records: &[ManagedAgentRecord],
    personas: &[AgentDefinition],
    pinned_id: Option<&str>,
) -> Result<UpdatePersonaRequest, String> {
    let EditAuthority {
        config,
        owner,
        relay,
    } = *authority;
    let grant = matching_grant(config, source, owner).ok_or("Agent edit permission was revoked")?;
    let AgentManagementDelegationPayload::V4(scoped) = &grant.payload else {
        return Err("Choose allowed targets and fields before delegating agent edits".into());
    };
    if scoped.relay_scope != relay || !scoped.grant.capabilities.agent_config {
        return Err("Agent edit permission does not apply in this community".into());
    }
    if payload.get("action").and_then(Value::as_str) != Some("update") {
        return Err("Scoped edit requires an update request".into());
    }
    let request = payload
        .get("request")
        .and_then(Value::as_object)
        .ok_or("Invalid edit request")?;
    let name = request
        .get("agentName")
        .and_then(Value::as_str)
        .ok_or("Missing target name")?;
    let candidates = personas
        .iter()
        .filter(|persona| {
            pinned_id.map_or_else(
                || {
                    persona
                        .display_name
                        .trim()
                        .eq_ignore_ascii_case(name.trim())
                },
                |id| persona.id == id,
            )
        })
        .collect::<Vec<_>>();
    if candidates.len() != 1 {
        return Err("Agent edit target must resolve to exactly one definition".into());
    }
    let persona = candidates[0];
    if persona.is_builtin || persona.source_team.is_some() {
        return Err("This agent definition is not editable".into());
    }
    let targets = records
        .iter()
        .filter(|record| record.persona_id.as_deref() == Some(&persona.id))
        .collect::<Vec<_>>();
    if targets
        .iter()
        .any(|record| canonical_relay_scope(&record.relay_url) != relay)
    {
        return Err("Agent definition is also used in another community".into());
    }
    let fields = request
        .keys()
        .filter(|key| !matches!(key.as_str(), "agentName" | "channelId"))
        .cloned()
        .collect::<Vec<_>>();
    let target_keys = targets
        .iter()
        .map(|record| record.pubkey.clone())
        .collect::<Vec<_>>();
    if !scoped.agent_edit.allows(source, &target_keys, &fields) {
        return Err(
            "Agent edit exceeds the allowed targets, fields, or self-edit permission".into(),
        );
    }
    for field in &fields {
        if !matches!(field.as_str(), "displayName" | "systemPrompt") || !request[field].is_string()
        {
            return Err("Unsupported delegated edit field".into());
        }
    }
    // Reconstruct from the durable definition, not a stale full-form UI snapshot.
    // No delegated edit can overwrite unrelated model, access, portrait, or env data.
    serde_json::from_value(serde_json::json!({
        "id": persona.id,
        "displayName": request.get("displayName").cloned().unwrap_or_else(|| Value::String(persona.display_name.clone())),
        "systemPrompt": request.get("systemPrompt").cloned().unwrap_or_else(|| Value::String(persona.system_prompt.clone())),
        "description": persona.description,
        "runtime": persona.runtime, "model": persona.model, "provider": persona.provider,
        "namePool": persona.name_pool,
    })).map_err(|error| error.to_string())
}

pub(in crate::commands) fn begin_scoped_update(
    app: &AppHandle,
    state: &AppState,
    request_id: &str,
) -> Result<UpdatePersonaRequest, String> {
    let request_id = canonical_request_id(request_id)?;
    let owner = state.signing_keys()?.public_key();
    let relay = canonical_relay_scope(&crate::relay::relay_ws_url_with_override(state));
    let mut ledger = load_state(app)?;
    let entry = ledger
        .requests
        .get_mut(&request_id)
        .ok_or("Management request is not registered")?;
    let result = prepare_entry(
        entry,
        &EditAuthority {
            config: &load_global_agent_config(app)?,
            owner: &owner,
            relay: &relay,
        },
        &load_managed_agents(app)?,
        &load_personas(app)?,
    );
    save_state(app, &ledger)?;
    result
}

fn prepare_entry(
    entry: &mut RequestLedgerEntry,
    authority: &EditAuthority<'_>,
    records: &[ManagedAgentRecord],
    personas: &[AgentDefinition],
) -> Result<UpdatePersonaRequest, String> {
    let EditAuthority { owner, relay, .. } = *authority;
    if entry.status != RequestStatus::Applying
        || entry.execution_started
        || entry.capability != DelegatedCapability::AgentConfig
        || entry.owner_reviewed
    {
        return Err("Management request is not awaiting a delegated edit".into());
    }
    if entry.owner_scope.as_deref() != Some(owner.to_hex().as_str())
        || entry.relay_scope.as_deref() != Some(relay)
    {
        return Err("Management request belongs to a different owner or community".into());
    }
    let input = match authorize_payload(
        authority,
        &entry.source_pubkey,
        &entry.payload,
        records,
        personas,
        entry.target_persona_id.as_deref(),
    ) {
        Ok(input) => input,
        Err(reason) => {
            entry.status = RequestStatus::PendingDraft;
            entry.updated_at = now_iso();
            return Err(format!("needs_owner_review:{reason}"));
        }
    };
    // Persist before the first mutation. A crash leaves an uncertain outcome;
    // neither command replay nor startup recovery is allowed to reapply it.
    entry.execution_started = true;
    Ok(input)
}

#[cfg(test)]
#[path = "agent_management_scoped_update_tests.rs"]
mod tests;
