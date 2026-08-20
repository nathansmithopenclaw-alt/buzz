//! Owner-facing persist for the v2 standing-delegation grant.
//!
//! Grant writes are independent of the Agent-defaults Save bar: they overlay
//! the latest durable config under `managed_agents_store_lock`. The private
//! audit channel is created-once with a deterministic uuid5 before signing;
//! if create fails, nothing is signed.

use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppState,
    events,
    managed_agents::global_config::{
        agent_management_audit_channel_uuid, canonical_relay_scope,
        AgentManagementDelegationSubjects,
    },
    relay::{query_relay, relay_ws_url_with_override, submit_event_with_keys},
};

use super::global_agent_config::{write_agent_management_delegation, GlobalAgentConfigSaveResult};

const AUDIT_CHANNEL_NAME: &str = "Agent management";
const AUDIT_CHANNEL_ABOUT: &str = "Audit log for delegated agent-management actions.";

#[tauri::command]
pub async fn set_agent_management_delegation(
    capabilities: Option<AgentManagementDelegationSubjects>,
    app: AppHandle,
) -> Result<GlobalAgentConfigSaveResult, String> {
    let state = app.state::<AppState>();
    crate::managed_agents::require_fleet_host(&state)?;
    let owner_keys = state.signing_keys()?;

    let capabilities = match capabilities {
        Some(mut subjects) => {
            subjects.canonicalize()?;
            if subjects.is_empty() {
                None
            } else {
                Some(subjects)
            }
        }
        None => None,
    };

    let audit_channel_id = if capabilities.is_some() {
        let existing = existing_audit_channel_id(&app)?;
        Some(ensure_agent_management_audit_channel(&state, &owner_keys, existing.as_deref()).await?)
    } else {
        None
    };

    write_agent_management_delegation(capabilities, audit_channel_id, app).await
}

fn existing_audit_channel_id(app: &AppHandle) -> Result<Option<String>, String> {
    let config = crate::managed_agents::load_global_agent_config(app)?;
    Ok(config
        .agent_management_delegation
        .as_ref()
        .map(|grant| grant.payload.audit_channel_id().to_string()))
}

async fn ensure_agent_management_audit_channel(
    state: &AppState,
    owner_keys: &nostr::Keys,
    existing_id: Option<&str>,
) -> Result<String, String> {
    let channel_id = if let Some(existing) = existing_id {
        uuid::Uuid::parse_str(existing.trim())
            .map_err(|_| "invalid delegation audit channel UUID".to_string())?
            .to_string()
    } else {
        let owner_hex = owner_keys.public_key().to_hex();
        let scope = canonical_relay_scope(&relay_ws_url_with_override(state));
        agent_management_audit_channel_uuid(&owner_hex, &scope).to_string()
    };

    let creator_pubkey = owner_keys.public_key().to_hex();
    if channel_metadata(state, &channel_id).await?.is_none() {
        create_audit_channel(state, owner_keys, &creator_pubkey, &channel_id).await?;
        let mut found = false;
        for _ in 0..3 {
            if channel_metadata(state, &channel_id).await?.is_some() {
                found = true;
                break;
            }
        }
        if !found {
            return Err(
                "audit channel created but metadata not yet available; grant was not signed".into(),
            );
        }
    }

    if !owner_is_audit_member(state, &channel_id, &creator_pubkey).await? {
        let uuid = uuid::Uuid::parse_str(&channel_id)
            .map_err(|_| "invalid delegation audit channel UUID".to_string())?;
        let builder = events::build_join(uuid)?;
        submit_event_with_keys(builder, state, owner_keys, None).await?;
        state.mark_pending_owned_channel(&creator_pubkey, &channel_id);
    }

    Ok(channel_id)
}

async fn create_audit_channel(
    state: &AppState,
    owner_keys: &nostr::Keys,
    creator_pubkey: &str,
    channel_id: &str,
) -> Result<(), String> {
    let uuid = uuid::Uuid::parse_str(channel_id)
        .map_err(|_| "invalid delegation audit channel UUID".to_string())?;
    let builder = events::build_create_channel(
        uuid,
        AUDIT_CHANNEL_NAME,
        "private",
        "stream",
        Some(AUDIT_CHANNEL_ABOUT),
        None,
    )?;
    match submit_event_with_keys(builder, state, owner_keys, None).await {
        Ok(_) => {
            state.mark_pending_owned_channel(creator_pubkey, channel_id);
            Ok(())
        }
        Err(error) if is_duplicate_channel_rejection(&error) => {
            state.mark_pending_owned_channel(creator_pubkey, channel_id);
            Ok(())
        }
        Err(error) => Err(error),
    }
}

async fn channel_metadata(
    state: &AppState,
    channel_id: &str,
) -> Result<Option<nostr::Event>, String> {
    let events = query_relay(
        state,
        &[serde_json::json!({
            "kinds": [39000],
            "#d": [channel_id],
            "limit": 1
        })],
    )
    .await?;
    Ok(events.into_iter().next())
}

async fn owner_is_audit_member(
    state: &AppState,
    channel_id: &str,
    owner_hex: &str,
) -> Result<bool, String> {
    if state.is_pending_owned_channel(owner_hex, channel_id) {
        return Ok(true);
    }
    let events = query_relay(
        state,
        &[serde_json::json!({
            "kinds": [39002],
            "#d": [channel_id],
            "#p": [owner_hex],
            "limit": 1
        })],
    )
    .await?;
    Ok(!events.is_empty())
}

fn is_duplicate_channel_rejection(error: &str) -> bool {
    error.contains("relay rejected event:") && error.contains("duplicate: channel already exists")
}

#[cfg(test)]
mod tests {
    use super::is_duplicate_channel_rejection;

    #[test]
    fn duplicate_create_is_treated_as_reuse() {
        assert!(is_duplicate_channel_rejection(
            "relay rejected event: duplicate: channel already exists"
        ));
        assert!(!is_duplicate_channel_rejection("relay unreachable"));
    }
}
