use super::*;
use crate::managed_agents::global_config::{
    delegation_scope::{AgentEditScope, ScopedDelegationPayload},
    AgentManagementDelegationCapabilities, AgentManagementDelegationPayloadV1,
};

const RELAY: &str = "wss://relay.example.test";

fn fixture() -> (
    Keys,
    String,
    GlobalAgentConfig,
    AgentDefinition,
    Vec<ManagedAgentRecord>,
    Value,
) {
    let owner = Keys::generate();
    let source = Keys::generate().public_key().to_hex();
    let target = Keys::generate().public_key().to_hex();
    let persona: AgentDefinition = serde_json::from_value(serde_json::json!({
        "id": "worker", "display_name": "Worker", "system_prompt": "original instructions",
        "avatar_url": "https://example.test/portrait.png", "description": "preserve description",
        "runtime": "codex", "model": "current-model", "provider": "openai", "name_pool": ["Worker"],
        "env_vars": { "KEEP_ME": "test-only-value" }, "created_at": "2026-09-08", "updated_at": "2026-09-08"
    })).unwrap();
    let mut record = persona.clone().into_agent_record();
    record.pubkey = target.clone();
    record.persona_id = Some(persona.id.clone());
    record.relay_url = RELAY.into();
    let payload = AgentManagementDelegationPayload::V4(ScopedDelegationPayload {
        version: 4,
        relay_scope: RELAY.into(),
        grant: AgentManagementDelegationPayloadV1 {
            grantee_pubkey: source.clone(),
            audit_channel_id: "7c07e659-3610-42f4-9a5e-1e9973c09da9".into(),
            capabilities: AgentManagementDelegationCapabilities {
                agent_config: true,
                ..Default::default()
            },
        },
        agent_edit: AgentEditScope {
            target_pubkeys: vec![target],
            allowed_fields: vec!["displayName".into()],
            allow_self_edit: false,
        },
    });
    let mut grant = AgentManagementDelegationGrant {
        payload,
        signed_event: None,
    };
    sign(&owner, &mut grant);
    let mut config = GlobalAgentConfig::default();
    config
        .agent_management_delegations
        .insert(source.clone(), grant);
    let request = serde_json::json!({ "type": "agent_management_request", "action": "update", "request": {
        "agentName": "Worker", "channelId": "channel", "displayName": "Updated worker"
    }});
    (owner, source, config, persona, vec![record], request)
}

fn sign(owner: &Keys, grant: &mut AgentManagementDelegationGrant) {
    grant.payload.canonicalize().unwrap();
    grant.signed_event = Some(
        EventBuilder::new(
            Kind::Custom(30_078),
            serde_json::to_string(&grant.payload).unwrap(),
        )
        .sign_with_keys(owner)
        .unwrap()
        .as_json(),
    );
}

fn entry(owner: &Keys, source: &str, payload: Value) -> RequestLedgerEntry {
    let mut ledger = AgentManagementState::default();
    register_in_ledger(
        &mut ledger,
        "request".into(),
        NewLedgerRequest {
            source_pubkey: source.into(),
            capability: DelegatedCapability::AgentConfig,
            payload_sha256: payload_sha256(&payload).unwrap(),
            payload,
            authorized: true,
            audit_channel_id: None,
            rejection: None,
        },
    )
    .unwrap();
    let mut entry = ledger.requests.remove("request").unwrap();
    entry.owner_scope = Some(owner.public_key().to_hex());
    entry.relay_scope = Some(RELAY.into());
    entry.target_persona_id = Some("worker".into());
    entry
}

#[test]
fn scoped_update_uses_fresh_definition_and_cannot_reexecute_after_serialization() {
    let (owner, source, config, mut persona, records, request) = fixture();
    let owner_key = owner.public_key();
    let authority = EditAuthority {
        config: &config,
        owner: &owner_key,
        relay: RELAY,
    };
    let mut entry = entry(&owner, &source, request);
    // A simultaneous direct edit must not be replaced by the frontend snapshot.
    persona.model = Some("new-owner-model".into());
    let input = prepare_entry(
        &mut entry,
        &authority,
        &records,
        std::slice::from_ref(&persona),
    )
    .unwrap();
    assert_eq!(input.display_name, "Updated worker");
    assert_eq!(input.system_prompt, persona.system_prompt);
    assert_eq!(input.model, persona.model);
    assert_eq!(input.description, persona.description);
    assert!(input.avatar_edit.is_none() && input.env_vars.is_none() && input.behavior.is_none());
    assert!(entry.execution_started);
    let mut restored = serde_json::from_slice(&serde_json::to_vec(&entry).unwrap()).unwrap();
    assert!(prepare_entry(&mut restored, &authority, &records, &[persona]).is_err());
}

#[test]
fn scoped_update_rechecks_revocation_after_registration_and_returns_to_review() {
    let (owner, source, mut config, persona, records, request) = fixture();
    let mut entry = entry(&owner, &source, request);
    config.agent_management_delegations.remove(&source);
    let result = prepare_entry(
        &mut entry,
        &EditAuthority {
            config: &config,
            owner: &owner.public_key(),
            relay: RELAY,
        },
        &records,
        &[persona],
    );
    assert!(result.unwrap_err().starts_with("needs_owner_review:"));
    assert_eq!(entry.status, RequestStatus::PendingDraft);
    assert!(!entry.execution_started);
}

#[test]
fn scoped_update_checks_every_field_and_every_linked_agent() {
    let (owner, source, config, persona, records, request) = fixture();
    let owner_key = owner.public_key();
    let authority = EditAuthority {
        config: &config,
        owner: &owner_key,
        relay: RELAY,
    };
    for field in [
        "systemPrompt",
        "runtime",
        "envVars",
        "respondTo",
        "avatarUrl",
        "futureField",
    ] {
        let mut extra = request.clone();
        extra["request"][field] = Value::String("ungranted".into());
        assert!(
            authorize_payload(
                &authority,
                &source,
                &extra,
                &records,
                std::slice::from_ref(&persona),
                None
            )
            .is_err(),
            "{field}"
        );
    }
    let mut linked = records.clone();
    let mut sibling = records[0].clone();
    sibling.pubkey = Keys::generate().public_key().to_hex();
    linked.push(sibling);
    assert!(authorize_payload(
        &authority,
        &source,
        &request,
        &linked,
        std::slice::from_ref(&persona),
        None
    )
    .is_err());
}

#[test]
fn scoped_update_uses_workspace_authority_with_empty_or_stale_legacy_relay() {
    let (owner, source, config, persona, records, request) = fixture();
    for legacy_relay in ["", RELAY, "wss://old.example.test"] {
        let mut records = records.clone();
        records[0].relay_url = legacy_relay.into();
        let active_relay = crate::relay::effective_agent_relay_url(legacy_relay, RELAY);
        let mut pending = entry(&owner, &source, request.clone());
        let input = prepare_entry(
            &mut pending,
            &EditAuthority {
                config: &config,
                owner: &owner.public_key(),
                relay: &active_relay,
            },
            &records,
            std::slice::from_ref(&persona),
        )
        .unwrap();
        assert_eq!(input.display_name, "Updated worker");
        assert!(pending.execution_started);

        // A grant from this workspace still cannot authorize another one,
        // even when a record's old pin happens to name that other workspace.
        let result = authorize_payload(
            &EditAuthority {
                config: &config,
                owner: &owner.public_key(),
                relay: "wss://old.example.test",
            },
            &source,
            &request,
            &records,
            std::slice::from_ref(&persona),
            None,
        );
        assert_eq!(
            result.unwrap_err(),
            "Agent edit permission does not apply in this community"
        );
    }
}

#[test]
fn scoped_update_signature_covers_targets_fields_and_self_edit() {
    let (owner, source, config, _, _, _) = fixture();
    let grant = &config.agent_management_delegations[&source];
    let decoded: AgentManagementDelegationGrant =
        serde_json::from_slice(&serde_json::to_vec(grant).unwrap()).unwrap();
    decoded
        .validate_for_grantee_key(&source, &owner.public_key())
        .unwrap();
    for mutation in 0..3 {
        let mut forged = decoded.clone();
        let AgentManagementDelegationPayload::V4(p) = &mut forged.payload else {
            unreachable!()
        };
        match mutation {
            0 => p
                .agent_edit
                .target_pubkeys
                .push(Keys::generate().public_key().to_hex()),
            1 => p.agent_edit.allowed_fields.push("systemPrompt".into()),
            _ => p.agent_edit.allow_self_edit = true,
        }
        forged.payload.canonicalize().unwrap();
        assert!(forged.validate(&owner.public_key()).is_err());
    }
}

#[test]
fn scoped_update_rejects_owner_switch_and_relay_switch_before_mutation() {
    let (owner, source, config, persona, records, request) = fixture();
    for (owner_key, relay) in [
        (Keys::generate().public_key(), RELAY),
        (owner.public_key(), "wss://other.example.test"),
    ] {
        let mut entry = entry(&owner, &source, request.clone());
        assert!(prepare_entry(
            &mut entry,
            &EditAuthority {
                config: &config,
                owner: &owner_key,
                relay
            },
            &records,
            std::slice::from_ref(&persona)
        )
        .is_err());
        assert!(!entry.execution_started);
    }
}

#[test]
fn scoped_update_instruction_permission_is_independent_and_legacy_grants_need_review() {
    let (owner, source, mut config, persona, records, mut request) = fixture();
    request["request"]
        .as_object_mut()
        .unwrap()
        .remove("displayName");
    request["request"]["systemPrompt"] = Value::String("New instructions".into());
    let grant = config
        .agent_management_delegations
        .get_mut(&source)
        .unwrap();
    let AgentManagementDelegationPayload::V4(scoped) = &mut grant.payload else {
        unreachable!()
    };
    scoped.agent_edit.allowed_fields = vec!["systemPrompt".into()];
    sign(&owner, grant);
    let input = authorize_payload(
        &EditAuthority {
            config: &config,
            owner: &owner.public_key(),
            relay: RELAY,
        },
        &source,
        &request,
        &records,
        std::slice::from_ref(&persona),
        None,
    )
    .unwrap();
    assert_eq!(input.system_prompt, "New instructions");
    assert_eq!(input.display_name, persona.display_name);
    let grant = config
        .agent_management_delegations
        .get_mut(&source)
        .unwrap();
    let AgentManagementDelegationPayload::V4(scoped) = &grant.payload else {
        unreachable!()
    };
    grant.payload = AgentManagementDelegationPayload::V1(scoped.grant.clone());
    sign(&owner, grant);
    let mut registered = entry(&owner, &source, request);
    assert!(prepare_entry(
        &mut registered,
        &EditAuthority {
            config: &config,
            owner: &owner.public_key(),
            relay: RELAY
        },
        &records,
        &[persona]
    )
    .unwrap_err()
    .starts_with("needs_owner_review:"));
    assert!(!registered.execution_started);
}
