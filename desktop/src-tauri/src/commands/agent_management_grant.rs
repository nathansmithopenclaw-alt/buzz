//! Owner-facing persist for independently signed standing-delegation grants.
//!
//! Grant writes are independent of the Agent-defaults Save bar: they overlay
//! the latest durable config under `managed_agents_store_lock`. Existing audit
//! channels are preserved; a new private channel uses a deterministic uuid5.
//! If creation or ownership validation fails, nothing is signed.

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

/// NIP-29 discovery is relay-signed, not creator-signed. Establish ownership
/// from the relay's current admin roles for this exact audit channel. The
/// deterministic UUID alone is not authority: another user can pre-create it.
fn require_owner_audit_metadata(
    metadata: &nostr::Event,
    admins: &nostr::Event,
    channel_id: &str,
    owner: &nostr::PublicKey,
    allow_existing_public: bool,
) -> Result<(), String> {
    for (event, kind) in [(metadata, 39_000), (admins, 39_001)] {
        if event.kind != nostr::Kind::Custom(kind)
            || event.tags.identifier() != Some(channel_id)
            || event.verify().is_err()
        {
            return Err("invalid audit channel discovery; grant was not signed".into());
        }
    }
    if metadata.pubkey != admins.pubkey {
        return Err("audit channel discovery signers disagree; grant was not signed".into());
    }
    let private = metadata
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["private"]);
    let public = metadata.tags.iter().any(|tag| tag.as_slice() == ["public"]);
    if private == public
        || metadata.tags.iter().any(|tag| {
            let values = tag.as_slice();
            values.first().is_some_and(|name| name == "archived")
                && values.get(1).is_some_and(|value| value == "true")
        })
    {
        return Err(
            "audit channel must be active with valid visibility; grant was not signed".into(),
        );
    }
    if public && !allow_existing_public {
        return Err("new audit channel must be private; grant was not signed".into());
    }
    let owner_hex = owner.to_hex();
    if !admins.tags.iter().any(|tag| {
        let values = tag.as_slice();
        values.first().is_some_and(|name| name == "p")
            && values.get(1) == Some(&owner_hex)
            && values.get(2).is_some_and(|role| role == "owner")
    }) {
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
    // Existing owner-configured audit channels may be public. Keep their
    // visibility and signed proofs intact; only newly created channels require
    // the private default. Channel ownership is still checked in both cases.
    let allow_existing_public = existing_id.is_some() && metadata.is_some();
    if metadata.is_none() {
        create_audit_channel(state, owner_keys, &owner_hex, &channel_id).await?;
        metadata = wait_for_audit_metadata(state, &channel_id).await?;
        if metadata.is_none() {
            return Err(
                "audit channel created but metadata not yet available; grant was not signed".into(),
            );
        }
    }
    // Creation emits metadata before admin roles. Wait for the latter too;
    // a pending-owner UI hint must never bypass this authorization check.
    let mut admins = None;
    for attempt in 0..3 {
        admins = channel_discovery(state, &channel_id, 39_001).await?;
        if admins.is_some() {
            break;
        }
        if attempt < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
    }
    let admins = admins
        .ok_or("audit channel ownership not yet available; grant was not signed; try again")?;
    require_owner_audit_metadata(
        metadata.as_ref().expect("checked above"),
        &admins,
        &channel_id,
        &owner,
        allow_existing_public,
    )?;

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
    channel_discovery(state, channel_id, 39_000).await
}

async fn channel_discovery(
    state: &AppState,
    channel_id: &str,
    kind: u16,
) -> Result<Option<nostr::Event>, String> {
    let events = query_relay(
        state,
        &[serde_json::json!({
            "kinds": [kind],
            "#d": [channel_id],
            "limit": 1
        })],
    )
    .await?;
    Ok(events.into_iter().next())
}

fn is_duplicate_channel_rejection(error: &str) -> bool {
    error.contains("relay rejected event:") && error.contains("duplicate: channel already exists")
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_agent_management_audit_channel, existing_audit_channel_id,
        is_duplicate_channel_rejection, require_owner_audit_metadata, resolve_audit_channel_id,
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
            .tags([
                Tag::parse(["d", channel_id]).unwrap(),
                Tag::parse(["private"]).unwrap(),
            ])
            .sign_with_keys(keys)
            .unwrap()
    }

    fn admin_event(relay: &Keys, channel_id: &str, owner: &Keys, role: &str) -> nostr::Event {
        EventBuilder::new(Kind::Custom(39_001), "")
            .tags([
                Tag::parse(["d", channel_id]).unwrap(),
                Tag::parse(["p", &owner.public_key().to_hex(), role]).unwrap(),
            ])
            .sign_with_keys(relay)
            .unwrap()
    }

    #[test]
    fn relay_signed_audit_discovery_accepts_owner_role() {
        let relay = Keys::generate();
        let owner = Keys::generate();
        assert_ne!(relay.public_key(), owner.public_key());
        require_owner_audit_metadata(
            &metadata_event(&relay, AUDIT),
            &admin_event(&relay, AUDIT, &owner, "owner"),
            AUDIT,
            &owner.public_key(),
            false,
        )
        .unwrap();
    }

    #[test]
    fn audit_discovery_rejects_non_owner_and_wrong_channel_or_signer() {
        let relay = Keys::generate();
        let owner = Keys::generate();
        let other = Keys::generate();
        let metadata = metadata_event(&relay, AUDIT);
        for admins in [
            admin_event(&relay, AUDIT, &owner, "admin"),
            admin_event(&relay, AUDIT, &owner, "member"),
            admin_event(&relay, AUDIT, &other, "owner"),
            admin_event(&relay, "different-channel", &owner, "owner"),
            admin_event(&other, AUDIT, &owner, "owner"),
            metadata.clone(),
        ] {
            assert!(require_owner_audit_metadata(
                &metadata,
                &admins,
                AUDIT,
                &owner.public_key(),
                true
            )
            .is_err());
        }
        let mut tampered = admin_event(&relay, AUDIT, &other, "owner");
        tampered.tags = admin_event(&relay, AUDIT, &owner, "owner").tags;
        assert!(require_owner_audit_metadata(
            &metadata,
            &tampered,
            AUDIT,
            &owner.public_key(),
            true
        )
        .is_err());
    }

    #[test]
    fn audit_discovery_rejects_archived_and_mismatched_metadata() {
        let relay = Keys::generate();
        let owner = Keys::generate();
        let admins = admin_event(&relay, AUDIT, &owner, "owner");
        for tags in [
            vec![vec!["d", AUDIT]],
            vec![vec!["d", AUDIT], vec!["private"], vec!["public"]],
            vec![vec!["d", AUDIT], vec!["private"], vec!["archived", "true"]],
            vec![vec!["d", AUDIT], vec!["public"], vec!["archived", "true"]],
            vec![vec!["d", "different-channel"], vec!["private"]],
        ] {
            let metadata = EventBuilder::new(Kind::Custom(39_000), "")
                .tags(tags.into_iter().map(|tag| Tag::parse(tag).unwrap()))
                .sign_with_keys(&relay)
                .unwrap();
            for allow_public in [false, true] {
                assert!(require_owner_audit_metadata(
                    &metadata,
                    &admins,
                    AUDIT,
                    &owner.public_key(),
                    allow_public
                )
                .is_err());
            }
        }
    }

    // Exercise the actual ensure path and HTTP query shape with relay-signed
    // NIP-29 discovery, including create -> metadata -> delayed admin emission.
    // No owner credentials, live relay, or durable grants are involved.
    #[tokio::test]
    async fn ensure_audit_channel_uses_relay_roles_for_existing_and_new_channels() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (create, owns_channel, public) in [
            (false, true, false),
            (true, true, false),
            (false, false, false),
            (false, true, true),
            (false, false, true),
            (true, true, true),
        ] {
            let relay = Keys::generate();
            let owner = Keys::generate();
            let metadata = EventBuilder::new(Kind::Custom(39_000), "{}")
                .tags([
                    Tag::parse(["d", AUDIT]).unwrap(),
                    Tag::parse([if public { "public" } else { "private" }]).unwrap(),
                ])
                .sign_with_keys(&relay)
                .unwrap();
            let other = Keys::generate();
            let admins = admin_event(
                &relay,
                AUDIT,
                if owns_channel { &owner } else { &other },
                "owner",
            );
            let mut replies = vec![];
            if create {
                replies.push(("/query", Some(39_000), "[]".to_string()));
                replies.push((
                    "/events",
                    None,
                    r#"{"accepted":true,"event_id":"test-create","message":""}"#.to_string(),
                ));
            }
            replies.push((
                "/query",
                Some(39_000),
                serde_json::to_string(&vec![metadata]).unwrap(),
            ));
            replies.push(("/query", Some(39_001), "[]".to_string()));
            replies.push((
                "/query",
                Some(39_001),
                serde_json::to_string(&vec![admins]).unwrap(),
            ));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                for (path, kind, body) in replies {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    let mut buf = [0; 4096];
                    loop {
                        let n = stream.read(&mut buf).await.unwrap();
                        assert_ne!(n, 0);
                        request.extend_from_slice(&buf[..n]);
                        if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                            let headers = String::from_utf8_lossy(&request[..end]);
                            let length: usize = headers
                                .lines()
                                .find_map(|line| {
                                    line.to_ascii_lowercase()
                                        .strip_prefix("content-length: ")
                                        .map(|v| v.parse().unwrap())
                                })
                                .unwrap();
                            if request.len() >= end + 4 + length {
                                assert!(headers.starts_with(&format!("POST {path} HTTP/1.1")));
                                let json: serde_json::Value =
                                    serde_json::from_slice(&request[end + 4..]).unwrap();
                                if let Some(kind) = kind {
                                    assert_eq!(
                                        json,
                                        serde_json::json!([{"kinds":[kind],"#d":[AUDIT],"limit":1}])
                                    );
                                }
                                break;
                            }
                        }
                    }
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    stream.write_all(response.as_bytes()).await.unwrap();
                }
            });
            let state = crate::app_state::build_app_state();
            *state.keys.lock().unwrap() = owner.clone();
            if !owns_channel {
                state.mark_pending_owned_channel(&owner.public_key().to_hex(), AUDIT);
            }
            *state.relay_url_override.lock().unwrap() = Some(format!("http://{addr}"));
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                ensure_agent_management_audit_channel(&state, &owner, Some(AUDIT)),
            )
            .await
            .unwrap();
            if create && public {
                assert!(result
                    .unwrap_err()
                    .contains("new audit channel must be private"));
            } else if owns_channel {
                assert_eq!(result.unwrap(), AUDIT);
            } else {
                assert!(result
                    .unwrap_err()
                    .contains("not owned by this Desktop owner"));
            }
            server.await.unwrap();
        }
    }
}
