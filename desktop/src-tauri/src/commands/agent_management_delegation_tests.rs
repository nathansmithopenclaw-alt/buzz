use super::*;

fn request(authorized: bool) -> NewLedgerRequest {
    NewLedgerRequest {
        source_pubkey: "11".repeat(32),
        capability: DelegatedCapability::AgentCreation,
        payload_sha256: "22".repeat(32),
        payload: serde_json::json!({"action": "create"}),
        authorized,
        audit_channel_id: Some("7c07e659-3610-42f4-9a5e-1e9973c09da9".into()),
    }
}

#[test]
fn grant_flip_routes_apply_or_durable_draft() {
    let mut applying = AgentManagementState::default();
    let applied = register_in_ledger(&mut applying, "request-a".into(), request(true)).unwrap();
    assert!(matches!(
        applied.disposition,
        ManagementRequestDisposition::Apply
    ));
    assert_eq!(
        applying.requests["request-a"].status,
        RequestStatus::Applying
    );

    let mut draft = AgentManagementState::default();
    let drafted = register_in_ledger(&mut draft, "request-b".into(), request(false)).unwrap();
    assert!(matches!(
        drafted.disposition,
        ManagementRequestDisposition::Draft
    ));
    assert_eq!(
        draft.requests["request-b"].status,
        RequestStatus::PendingDraft
    );
}

#[test]
fn applying_replay_survives_serialization_without_reminting() {
    let mut before_restart = AgentManagementState::default();
    register_in_ledger(&mut before_restart, "request-a".into(), request(true)).unwrap();
    let bytes = serde_json::to_vec(&before_restart).unwrap();
    let mut after_restart: AgentManagementState = serde_json::from_slice(&bytes).unwrap();

    let replay = register_in_ledger(&mut after_restart, "request-a".into(), request(true)).unwrap();
    assert!(matches!(
        replay.disposition,
        ManagementRequestDisposition::DuplicateApplying
    ));
    assert_eq!(after_restart.requests.len(), 1);
}

#[test]
fn pending_drafts_restore_in_creation_order() {
    let mut ledger = AgentManagementState::default();
    register_in_ledger(&mut ledger, "request-b".into(), request(false)).unwrap();
    register_in_ledger(&mut ledger, "request-a".into(), request(false)).unwrap();
    ledger.requests.get_mut("request-b").unwrap().created_at = "2026-08-04T00:00:00Z".into();
    ledger.requests.get_mut("request-a").unwrap().created_at = "2026-08-04T00:00:01Z".into();
    assert_eq!(
        pending_requests(ledger)
            .into_iter()
            .map(|request| request.request_id)
            .collect::<Vec<_>>(),
        ["request-b", "request-a"]
    );
}

#[test]
fn owner_review_transition_is_distinct_from_standing_grant_apply() {
    let mut ledger = AgentManagementState::default();
    register_in_ledger(&mut ledger, "request-a".into(), request(false)).unwrap();
    begin_owner_reviewed_in_ledger(&mut ledger, "request-a").unwrap();
    let entry = &ledger.requests["request-a"];
    assert_eq!(entry.status, RequestStatus::Applying);
    assert!(entry.owner_reviewed);
    assert!(begin_owner_reviewed_in_ledger(&mut ledger, "request-a").is_err());
}

#[test]
fn request_uuid_collision_is_rejected() {
    let mut ledger = AgentManagementState::default();
    register_in_ledger(&mut ledger, "request-a".into(), request(true)).unwrap();
    let mut collision = request(true);
    collision.payload_sha256 = "33".repeat(32);
    assert!(register_in_ledger(&mut ledger, "request-a".into(), collision).is_err());
}

#[test]
fn ledger_result_rejects_nested_secret_fields() {
    assert!(value_contains_secret(&serde_json::json!({
        "safe": {"privateKey": "secret"}
    })));
    assert!(!value_contains_secret(&serde_json::json!({
        "pubkey": "11"
    })));
}

#[test]
fn consolidated_ipc_contract_accepts_camel_case_fields() {
    let command: AgentManagementLedgerCommand = serde_json::from_value(serde_json::json!({
        "action": "register",
        "sourcePubkey": "11".repeat(32),
        "requestId": "b2f095a4-664b-4b41-a6fd-4460fa38e520",
        "capability": "agent_creation",
        "intent": "apply",
        "payload": {"action": "create"}
    }))
    .unwrap();
    assert!(matches!(
        command,
        AgentManagementLedgerCommand::Register {
            capability: DelegatedCapability::AgentCreation,
            ..
        }
    ));

    let publish: AgentManagementPublishCommand = serde_json::from_value(serde_json::json!({
        "action": "create_secret",
        "requestId": "b2f095a4-664b-4b41-a6fd-4460fa38e520",
        "sourcePubkey": "11".repeat(32),
        "agentPubkey": "22".repeat(32),
        "privateKeyNsec": "nsec1example"
    }))
    .unwrap();
    assert!(matches!(
        publish,
        AgentManagementPublishCommand::CreateSecret { .. }
    ));
}
