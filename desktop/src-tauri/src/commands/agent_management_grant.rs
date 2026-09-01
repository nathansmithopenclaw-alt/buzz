//! Owner-facing persist for independently signed standing-delegation grants.
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
        AgentManagementDelegationSubjects, GlobalAgentConfig,
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
        let config = crate::managed_agents::load_global_agent_config(&app)?;
        let existing = existing_audit_channel_id(&config);
        Some(ensure_agent_management_audit_channel(&state, &owner_keys, existing.as_deref()).await?)
    } else {
        None
    };

    write_agent_management_delegation(capabilities, audit_channel_id, app).await
}

fn existing_audit_channel_id(config: &GlobalAgentConfig) -> Option<String> {
    config
        .agent_management_delegation
        .as_ref()
        .map(|grant| grant.payload.audit_channel_id().to_string())
        .or_else(|| {
            config
                .agent_management_delegations
                .values()
                .next()
                .map(|grant| grant.payload.audit_channel_id().to_string())
        })
}

fn resolve_audit_channel_id(
    existing_id: Option<&str>,
    owner_hex: &str,
    relay_scope: &str,
) -> Result<String, String> {
    if let Some(existing) = existing_id {
        return uuid::Uuid::parse_str(existing.trim())
            .map(|uuid| uuid.to_string())
            .map_err(|_| "invalid delegation audit channel UUID".to_string());
    }
    Ok(agent_management_audit_channel_uuid(owner_hex, relay_scope).to_string())
}

/// Fail closed if a kind:39000 for this id exists but was not created by the
/// current Desktop owner. The uuid5 inputs are public, so a pre-created
/// channel on a multi-tenant relay is otherwise a squat.
fn require_owner_audit_metadata(
    metadata: &nostr::Event,
    owner: &nostr::PublicKey,
) -> Result<(), String> {
    if metadata.pubkey != *owner {
        return Err(
            "audit channel exists but is not owned by this Desktop owner; grant was not signed"
                .into(),
        );
    }
    Ok(())
}

async fn ensure_agent_management_audit_channel(
    state: &AppState,
    owner_keys: &nostr::Keys,
    existing_id: Option<&str>,
) -> Result<String, String> {
    let owner = owner_keys.public_key();
    let owner_hex = owner.to_hex();
    let channel_id = resolve_audit_channel_id(
        existing_id,
        &owner_hex,
        &canonical_relay_scope(&relay_ws_url_with_override(state)),
    )?;

    let mut metadata = channel_metadata(state, &channel_id).await?;
    if metadata.is_none() {
        create_audit_channel(state, owner_keys, &owner_hex, &channel_id).await?;
        metadata = wait_for_audit_metadata(state, &channel_id).await?;
        if metadata.is_none() {
            return Err(
                "audit channel created but metadata not yet available; grant was not signed".into(),
            );
        }
    }
    require_owner_audit_metadata(metadata.as_ref().expect("checked above"), &owner)?;

    if !owner_is_audit_member(state, &channel_id, &owner_hex).await? {
        let uuid = uuid::Uuid::parse_str(&channel_id)
            .map_err(|_| "invalid delegation audit channel UUID".to_string())?;
        let builder = events::build_join(uuid)?;
        submit_event_with_keys(builder, state, owner_keys, None).await?;
        state.mark_pending_owned_channel(&owner_hex, &channel_id);
    }

    Ok(channel_id)
}

async fn wait_for_audit_metadata(
    state: &AppState,
    channel_id: &str,
) -> Result<Option<nostr::Event>, String> {
    let mut found = None;
    for attempt in 0..3 {
        found = channel_metadata(state, channel_id).await?;
        if found.is_some() {
            break;
        }
        if attempt + 1 < 3 {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
    }
    Ok(found)
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
        Err(error) if is_duplicate_channel_rejection(&error) => Ok(()),
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
    use super::{
        existing_audit_channel_id, is_duplicate_channel_rejection, require_owner_audit_metadata,
        resolve_audit_channel_id,
    };
    use crate::managed_agents::{
        global_config::{
            agent_management_audit_channel_uuid, AgentManagementDelegationPayload,
            AgentManagementDelegationPayloadV2, AgentManagementDelegationSubjects,
        },
        AgentManagementDelegationGrant, GlobalAgentConfig,
    };
    use nostr::{EventBuilder, Keys, Kind, Tag};

    const AUDIT: &str = "7c07e659-3610-42f4-9a5e-1e9973c09da9";

    #[test]
    fn duplicate_create_is_treated_as_reuse() {
        assert!(is_duplicate_channel_rejection(
            "relay rejected event: duplicate: channel already exists"
        ));
        assert!(!is_duplicate_channel_rejection("relay unreachable"));
    }

    #[test]
    fn existing_audit_channel_id_reads_the_grant_and_none_without_one() {
        assert_eq!(
            existing_audit_channel_id(&GlobalAgentConfig::default()),
            None
        );
        let config = GlobalAgentConfig {
            agent_management_delegation: Some(AgentManagementDelegationGrant {
                payload: AgentManagementDelegationPayload::V2(AgentManagementDelegationPayloadV2 {
                    version: 2,
                    audit_channel_id: AUDIT.into(),
                    capabilities: AgentManagementDelegationSubjects::all_agents(),
                }),
                signed_event: None,
            }),
            ..Default::default()
        };
        assert_eq!(existing_audit_channel_id(&config).as_deref(), Some(AUDIT));

        let grantee = "11".repeat(32);
        let keyed = GlobalAgentConfig {
            agent_management_delegations: [(
                grantee.clone(),
                AgentManagementDelegationGrant {
                    payload: AgentManagementDelegationPayload::V1(
                        crate::managed_agents::global_config::AgentManagementDelegationPayloadV1 {
                            grantee_pubkey: grantee,
                            audit_channel_id: AUDIT.into(),
                            capabilities: Default::default(),
                        },
                    ),
                    signed_event: None,
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        assert_eq!(existing_audit_channel_id(&keyed).as_deref(), Some(AUDIT));
    }

    #[test]
    fn resolve_audit_channel_id_reuses_existing_and_derives_once_otherwise() {
        let owner = "aa".repeat(32);
        let scope = "wss://relay.example:8443";
        let derived = resolve_audit_channel_id(None, &owner, scope).unwrap();
        assert_eq!(
            derived,
            agent_management_audit_channel_uuid(&owner, scope).to_string()
        );
        assert_eq!(
            resolve_audit_channel_id(None, &owner, scope).unwrap(),
            derived
        );
        assert_eq!(
            resolve_audit_channel_id(Some(AUDIT), &owner, scope).unwrap(),
            AUDIT
        );
        assert!(resolve_audit_channel_id(Some("not-a-uuid"), &owner, scope).is_err());
    }

    fn metadata_event(keys: &Keys, channel_id: &str) -> nostr::Event {
        EventBuilder::new(Kind::Custom(39_000), "{}")
            .tags([Tag::parse(["d", channel_id]).unwrap()])
            .sign_with_keys(keys)
            .unwrap()
    }

    #[test]
    fn require_owner_audit_metadata_reuses_owner_and_fails_closed_on_squat() {
        let owner = Keys::generate();
        let attacker = Keys::generate();
        require_owner_audit_metadata(&metadata_event(&owner, AUDIT), &owner.public_key()).unwrap();
        let err =
            require_owner_audit_metadata(&metadata_event(&attacker, AUDIT), &owner.public_key())
                .unwrap_err();
        assert!(err.contains("not owned by this Desktop owner"));
        assert!(err.contains("grant was not signed"));
    }
}
