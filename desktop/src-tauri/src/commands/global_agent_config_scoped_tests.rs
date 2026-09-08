use super::*;
use crate::managed_agents::global_config::{
    delegation_scope::AgentEditScope, AgentManagementDelegationPayload,
    AgentManagementDelegationSubjects, CapabilitySubject,
};

#[test]
fn scoped_grant_persist_signs_each_recipient_and_preserves_unrelated_proofs() {
    let owner = nostr::Keys::generate();
    let first = nostr::Keys::generate().public_key().to_hex();
    let second = nostr::Keys::generate().public_key().to_hex();
    let target = nostr::Keys::generate().public_key().to_hex();
    let roster = vec![first.clone(), second.clone(), target.clone()];
    let subjects = AgentManagementDelegationSubjects {
        agent_config: CapabilitySubject::Set {
            grantees: vec![first.clone(), second.clone()],
        },
        ..Default::default()
    };
    let config = apply_delegation_subjects(
        GlobalAgentConfig::default(),
        Some(subjects.clone()),
        Some("7c07e659-3610-42f4-9a5e-1e9973c09da9".into()),
        &roster,
        &owner,
    )
    .unwrap();
    let scope = AgentEditScope {
        target_pubkeys: vec![target],
        allowed_fields: vec!["displayName".into()],
        allow_self_edit: false,
    };
    let config = apply_edit_scopes(
        config,
        [
            (first.clone(), scope.clone()),
            (second.clone(), scope.clone()),
        ]
        .into(),
        &roster,
        &owner,
        "wss://relay.example.test",
    )
    .unwrap();
    for grantee in [&first, &second] {
        config.agent_management_delegations[grantee]
            .validate_for_grantee_key(grantee, &owner.public_key())
            .unwrap();
        assert!(matches!(
            config.agent_management_delegations[grantee].payload,
            AgentManagementDelegationPayload::V4(_)
        ));
    }
    let first_proof = config.agent_management_delegations[&first]
        .signed_event
        .clone();
    let second_proof = config.agent_management_delegations[&second]
        .signed_event
        .clone();
    let config = apply_delegation_subjects(
        config,
        Some(subjects),
        Some("7c07e659-3610-42f4-9a5e-1e9973c09da9".into()),
        &roster,
        &owner,
    )
    .unwrap();
    assert_eq!(
        config.agent_management_delegations[&first].signed_event,
        first_proof
    );
    assert_eq!(
        config.agent_management_delegations[&second].signed_event,
        second_proof
    );
    let mut changed = scope.clone();
    changed.allowed_fields.push("systemPrompt".into());
    let updated = apply_edit_scopes(
        config.clone(),
        [(second.clone(), changed)].into(),
        &roster,
        &owner,
        "wss://relay.example.test",
    )
    .unwrap();
    assert_eq!(
        updated.agent_management_delegations[&first].signed_event,
        first_proof
    );
    assert_ne!(
        updated.agent_management_delegations[&second].signed_event,
        second_proof
    );
    let mut invalid = scope;
    invalid.allowed_fields.push("envVars".into());
    assert!(apply_edit_scopes(
        config,
        [(first, invalid)].into(),
        &roster,
        &owner,
        "wss://relay.example.test"
    )
    .is_err());
}
