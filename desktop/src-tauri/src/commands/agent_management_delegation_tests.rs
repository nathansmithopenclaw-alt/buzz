use super::*;
use crate::managed_agents::global_config::AgentManagementDelegationCapabilities;
use nostr::ToBech32;

fn unsigned_grant(
    grantee_pubkey: String,
    capabilities: AgentManagementDelegationCapabilities,
) -> AgentManagementDelegationGrant {
    AgentManagementDelegationGrant {
        payload: crate::managed_agents::global_config::AgentManagementDelegationPayload {
            grantee_pubkey,
            audit_channel_id: "7c07e659-3610-42f4-9a5e-1e9973c09da9".into(),
            capabilities,
        },
        signed_event: None,
    }
}

fn request(authorized: bool) -> NewLedgerRequest {
    NewLedgerRequest {
        source_pubkey: "11".repeat(32),
        capability: DelegatedCapability::AgentCreation,
        payload_sha256: "22".repeat(32),
        payload: serde_json::json!({"action": "create"}),
        authorized,
        audit_channel_id: Some("7c07e659-3610-42f4-9a5e-1e9973c09da9".into()),
        rejection: None,
    }
}

fn mark_applied(ledger: &mut AgentManagementState, request_id: &str, pubkey: String) {
    let entry = ledger.requests.get_mut(request_id).unwrap();
    entry.status = RequestStatus::Applied;
    entry.result = Some(serde_json::json!({"pubkey": pubkey}));
}

#[test]
fn delegated_env_rejection_is_durable_and_replays_without_apply() {
    let mut ledger = AgentManagementState::default();
    let mut rejected = request(true);
    rejected.capability = DelegatedCapability::EnvVars;
    rejected.payload = serde_json::json!({
        "action": "update",
        "request": {"envVars": {"BUZZ_PRIVATE_KEY": "must-never-leak"}}
    });
    rejected.rejection = Some(
        "the following env vars are reserved by Buzz and cannot be overridden: BUZZ_PRIVATE_KEY"
            .into(),
    );
    let result = register_in_ledger(&mut ledger, "request-env".into(), rejected).unwrap();
    assert!(matches!(
        result.disposition,
        ManagementRequestDisposition::Rejected
    ));
    assert_eq!(ledger.requests["request-env"].status, RequestStatus::Failed);
    assert_eq!(result.result.unwrap()["code"], "invalid_env_vars");

    let mut replay = request(true);
    replay.capability = DelegatedCapability::EnvVars;
    replay.payload = ledger.requests["request-env"].payload.clone();
    replay.rejection = Some("different transient text is ignored on replay".into());
    let duplicate = register_in_ledger(&mut ledger, "request-env".into(), replay).unwrap();
    assert!(matches!(
        duplicate.disposition,
        ManagementRequestDisposition::DuplicateFailed
    ));
}

#[test]
fn env_fingerprint_summary_never_contains_values() {
    let payload = serde_json::json!({
        "request": {"envVars": {"OPENAI_API_KEY": "dummy-secret", "OPENAI_HOST": "http://lane"}}
    });
    let summary = env_fingerprint_summary(&payload).unwrap();
    assert!(summary.contains("OPENAI_API_KEY=sha256:"));
    assert!(summary.contains("OPENAI_HOST=sha256:"));
    assert!(!summary.contains("dummy-secret"));
    assert!(!summary.contains("http://lane"));
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
fn agent_and_global_config_grants_are_independent_and_env_is_conjunctive() {
    let source = "11".repeat(32);
    let grant = unsigned_grant(
        source.clone(),
        AgentManagementDelegationCapabilities {
            agent_config: true,
            global_config: false,
            teams: false,
            agent_creation: false,
            env_vars: false,
        },
    );

    assert!(grant_authorizes_request(
        &grant,
        &source,
        DelegatedCapability::AgentConfig,
        false,
    ));
    assert!(!grant_authorizes_request(
        &grant,
        &source,
        DelegatedCapability::GlobalConfig,
        false,
    ));
    assert!(!grant_authorizes_request(
        &grant,
        &source,
        DelegatedCapability::AgentConfig,
        true,
    ));

    let globally_scoped = unsigned_grant(
        source.clone(),
        AgentManagementDelegationCapabilities {
            global_config: true,
            env_vars: true,
            ..Default::default()
        },
    );
    assert!(grant_authorizes_request(
        &globally_scoped,
        &source,
        DelegatedCapability::GlobalConfig,
        true,
    ));
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
fn create_secret_response_is_owner_signed_and_reuses_exact_ciphertext() {
    let owner = Keys::generate();
    let source = Keys::generate();
    let agent = Keys::generate();
    let request_id = "b2f095a4-664b-4b41-a6fd-4460fa38e520";
    let mut ledger = AgentManagementState::default();
    let mut applied = request(true);
    applied.source_pubkey = source.public_key().to_hex();
    register_in_ledger(&mut ledger, request_id.into(), applied).unwrap();
    mark_applied(&mut ledger, request_id, agent.public_key().to_hex());

    let (first, created) = retained_agent_create_secret_event(
        &mut ledger,
        request_id,
        &source.public_key(),
        &agent,
        &agent.secret_key().to_bech32().unwrap(),
        &owner,
    )
    .unwrap();
    assert!(created);
    assert_eq!(first.kind, Kind::Custom(30_078));
    assert_eq!(first.pubkey, owner.public_key());

    let (replay, created_again) = retained_agent_create_secret_event(
        &mut ledger,
        request_id,
        &source.public_key(),
        &agent,
        &agent.secret_key().to_bech32().unwrap(),
        &owner,
    )
    .unwrap();
    assert!(!created_again);
    assert_eq!(replay.id, first.id);
    assert_eq!(replay.content, first.content);
}

#[test]
fn create_secret_response_rejects_wrong_request_authority_or_agent() {
    let owner = Keys::generate();
    let source = Keys::generate();
    let wrong_source = Keys::generate();
    let agent = Keys::generate();
    let wrong_agent = Keys::generate();
    let request_id = "b2f095a4-664b-4b41-a6fd-4460fa38e520";
    let mut ledger = AgentManagementState::default();
    let mut applied = request(true);
    applied.source_pubkey = source.public_key().to_hex();
    register_in_ledger(&mut ledger, request_id.into(), applied).unwrap();
    mark_applied(&mut ledger, request_id, agent.public_key().to_hex());

    for (candidate_source, candidate_agent) in [
        (wrong_source.public_key(), &agent),
        (source.public_key(), &wrong_agent),
    ] {
        assert_eq!(
            retained_agent_create_secret_event(
                &mut ledger,
                request_id,
                &candidate_source,
                candidate_agent,
                &candidate_agent.secret_key().to_bech32().unwrap(),
                &owner,
            )
            .unwrap_err(),
            "create secret response does not match the applied request"
        );
    }
    assert!(ledger.requests[request_id].secret_response_event.is_none());
}

#[test]
fn legacy_ledger_entries_default_to_result_already_published() {
    let entry: RequestLedgerEntry = serde_json::from_value(serde_json::json!({
        "sourcePubkey": "11".repeat(32),
        "capability": "agent_creation",
        "payloadSha256": "22".repeat(32),
        "payload": {"action": "create"},
        "status": "failed",
        "createdAt": "2026-08-04T00:00:00Z",
        "updatedAt": "2026-08-04T00:00:01Z",
        "result": {"error": "legacy failure"}
    }))
    .unwrap();

    assert!(entry.result_published);
}

#[test]
fn startup_restore_fails_applying_and_keeps_drafts_in_creation_order() {
    let mut ledger = AgentManagementState::default();
    register_in_ledger(&mut ledger, "request-b".into(), request(false)).unwrap();
    register_in_ledger(&mut ledger, "request-a".into(), request(true)).unwrap();
    ledger.requests.get_mut("request-b").unwrap().created_at = "2026-08-04T00:00:00Z".into();
    ledger.requests.get_mut("request-a").unwrap().created_at = "2026-08-04T00:00:01Z".into();
    ledger.requests.get_mut("request-a").unwrap().updated_at = "2026-08-04T00:00:01Z".into();
    let startup_cutoff = chrono::DateTime::parse_from_rfc3339("2026-08-04T00:00:02Z").unwrap();
    let (restored, changed) = restore_requests(&mut ledger, startup_cutoff);
    assert!(changed);
    assert_eq!(
        restored
            .into_iter()
            .map(|request| (request.request_id, request.disposition))
            .collect::<Vec<_>>(),
        [
            (
                "request-b".to_string(),
                RestoredManagementRequestDisposition::Draft
            ),
            (
                "request-a".to_string(),
                RestoredManagementRequestDisposition::Failed
            )
        ]
    );
    let interrupted = &ledger.requests["request-a"];
    assert_eq!(interrupted.status, RequestStatus::Failed);
    assert_eq!(
        interrupted.result.as_ref().unwrap()["code"],
        "interrupted_during_apply"
    );
    assert!(interrupted.recovery_batch_id.is_some());

    let (_, changed_again) = restore_requests(&mut ledger, startup_cutoff);
    assert!(!changed_again);
    ledger
        .requests
        .get_mut("request-a")
        .unwrap()
        .result_published = true;
    let (restored_with_audit_pending, _) = restore_requests(&mut ledger, startup_cutoff);
    assert_eq!(restored_with_audit_pending.len(), 2);
    ledger
        .requests
        .get_mut("request-a")
        .unwrap()
        .audit_published = true;
    let (restored_after_publish, _) = restore_requests(&mut ledger, startup_cutoff);
    assert_eq!(restored_after_publish.len(), 1);
    assert_eq!(restored_after_publish[0].request_id, "request-b");
}

#[test]
fn startup_restore_does_not_fail_an_apply_registered_after_the_cutoff() {
    let mut ledger = AgentManagementState::default();
    register_in_ledger(&mut ledger, "request-a".into(), request(true)).unwrap();
    ledger.requests.get_mut("request-a").unwrap().updated_at = "2026-08-04T00:00:03Z".into();
    let cutoff = chrono::DateTime::parse_from_rfc3339("2026-08-04T00:00:02Z").unwrap();

    let (restored, changed) = restore_requests(&mut ledger, cutoff);

    assert!(!changed);
    assert!(restored.is_empty());
    assert_eq!(ledger.requests["request-a"].status, RequestStatus::Applying);
}

#[test]
fn startup_restore_groups_fanned_out_failures_into_one_owner_signed_receipt() {
    let owner = Keys::generate();
    let source = Keys::generate();
    let mut ledger = AgentManagementState::default();
    for index in 0..5 {
        let mut candidate = request(true);
        candidate.source_pubkey = source.public_key().to_hex();
        register_in_ledger(&mut ledger, format!("request-{index}"), candidate).unwrap();
        ledger
            .requests
            .get_mut(&format!("request-{index}"))
            .unwrap()
            .updated_at = "2026-08-04T00:00:01Z".into();
    }
    let cutoff = chrono::DateTime::parse_from_rfc3339("2026-08-04T00:00:02Z").unwrap();
    let (restored, changed) = restore_requests(&mut ledger, cutoff);
    assert!(changed);
    assert_eq!(restored.len(), 5);

    let (event, group_ids) = result_publication::retained_recovery_audit_event(
        &mut ledger,
        "request-0",
        &source.public_key(),
        "Requester",
        &owner,
    )
    .unwrap()
    .expect("recovery receipt");
    assert_eq!(group_ids.len(), 5);
    assert_eq!(event.pubkey, owner.public_key());
    assert_eq!(
        event
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().map(|value| value.as_str()) == Some("p"))
            .count(),
        1
    );
    assert!(event
        .content
        .contains("5 interrupted delegated management requests"));
    assert!(!event.content.contains("retry with a new request id"));
    let raw_event = event.as_json();
    assert!(ledger
        .requests
        .values()
        .all(|entry| entry.audit_event.as_deref() == Some(raw_event.as_str())));

    let replay = result_publication::retained_recovery_audit_event(
        &mut ledger,
        "request-4",
        &source.public_key(),
        "Requester",
        &owner,
    )
    .unwrap()
    .expect("stored receipt remains retryable until publish is recorded");
    assert_eq!(replay.0.id, event.id);
}

#[test]
fn recovery_audit_groups_do_not_cross_requester_or_channel_boundaries() {
    let owner = Keys::generate();
    let first_source = Keys::generate();
    let second_source = Keys::generate();
    let mut ledger = AgentManagementState::default();
    for (request_id, source, channel) in [
        ("request-a", &first_source, "channel-a"),
        ("request-b", &first_source, "channel-b"),
        ("request-c", &second_source, "channel-a"),
    ] {
        let mut candidate = request(true);
        candidate.source_pubkey = source.public_key().to_hex();
        candidate.audit_channel_id = Some(channel.into());
        register_in_ledger(&mut ledger, request_id.into(), candidate).unwrap();
        ledger.requests.get_mut(request_id).unwrap().updated_at = "2026-08-04T00:00:01Z".into();
    }
    let cutoff = chrono::DateTime::parse_from_rfc3339("2026-08-04T00:00:02Z").unwrap();
    restore_requests(&mut ledger, cutoff);

    let (_, group_ids) = result_publication::retained_recovery_audit_event(
        &mut ledger,
        "request-a",
        &first_source.public_key(),
        "Requester",
        &owner,
    )
    .unwrap()
    .expect("first recovery receipt");
    assert_eq!(group_ids, ["request-a".to_string()]);
}

#[test]
fn recovery_publish_response_must_match_the_signed_receipt_id() {
    let owner = Keys::generate();
    let event = EventBuilder::new(Kind::TextNote, "recovery")
        .sign_with_keys(&owner)
        .unwrap();
    let mismatched = crate::relay::SubmitEventResponse {
        event_id: "00".repeat(32),
        accepted: true,
        message: String::new(),
    };
    assert!(result_publication::verify_recovery_publish_response(&event, &mismatched).is_err());
    let matched = crate::relay::SubmitEventResponse {
        event_id: event.id.to_hex(),
        accepted: true,
        message: String::new(),
    };
    result_publication::verify_recovery_publish_response(&event, &matched).unwrap();
}

const RECOVERY_CRASH_LEDGER_ENV: &str = "BUZZ_TEST_RECOVERY_CRASH_LEDGER";
const RECOVERY_CRASH_MARKER_ENV: &str = "BUZZ_TEST_RECOVERY_CRASH_MARKER";

#[test]
fn recovery_claim_crash_child() {
    let Ok(ledger_path) = std::env::var(RECOVERY_CRASH_LEDGER_ENV) else {
        return;
    };
    let marker_path = std::env::var(RECOVERY_CRASH_MARKER_ENV).unwrap();
    let mut ledger: AgentManagementState =
        serde_json::from_slice(&std::fs::read(ledger_path).unwrap()).unwrap();
    let source = PublicKey::parse(&ledger.requests["request-0"].source_pubkey).unwrap();
    let owner = Keys::generate();
    result_publication::retained_recovery_audit_event(
        &mut ledger,
        "request-0",
        &source,
        "Requester",
        &owner,
    )
    .unwrap()
    .expect("volatile claim");
    // Signal only after signed bytes exist in memory. The parent kills this
    // process before any ledger write, directly exercising the pre-store gap.
    std::fs::write(marker_path, b"claimed").unwrap();
    loop {
        std::thread::park();
    }
}

#[test]
fn recovery_claim_survives_process_kill_before_store_and_publishes_once() {
    let owner = Keys::generate();
    let source = Keys::generate();
    let mut ledger = AgentManagementState::default();
    for index in 0..5 {
        let mut candidate = request(true);
        candidate.source_pubkey = source.public_key().to_hex();
        register_in_ledger(&mut ledger, format!("request-{index}"), candidate).unwrap();
        ledger
            .requests
            .get_mut(&format!("request-{index}"))
            .unwrap()
            .updated_at = "2026-08-04T00:00:01Z".into();
    }
    let cutoff = chrono::DateTime::parse_from_rfc3339("2026-08-04T00:00:02Z").unwrap();
    restore_requests(&mut ledger, cutoff);

    let directory = tempfile::tempdir().unwrap();
    let ledger_path = directory.path().join("ledger.json");
    let marker_path = directory.path().join("claimed");
    std::fs::write(&ledger_path, serde_json::to_vec(&ledger).unwrap()).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("recovery_claim_crash_child")
        .arg("--nocapture")
        .env(RECOVERY_CRASH_LEDGER_ENV, &ledger_path)
        .env(RECOVERY_CRASH_MARKER_ENV, &marker_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..250 {
        if marker_path.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        marker_path.exists(),
        "child never reached the pre-store crash point"
    );
    child.kill().unwrap();
    let status = child.wait().unwrap();
    assert!(!status.success());

    let mut restarted: AgentManagementState =
        serde_json::from_slice(&std::fs::read(&ledger_path).unwrap()).unwrap();
    assert!(restarted
        .requests
        .values()
        .all(|entry| entry.audit_event.is_none() && !entry.audit_published));
    let (event, group_ids) = result_publication::retained_recovery_audit_event(
        &mut restarted,
        "request-0",
        &source.public_key(),
        "Requester",
        &owner,
    )
    .unwrap()
    .expect("restart prepares the receipt");
    std::fs::write(&ledger_path, serde_json::to_vec(&restarted).unwrap()).unwrap();

    let mut after_store: AgentManagementState =
        serde_json::from_slice(&std::fs::read(&ledger_path).unwrap()).unwrap();
    let replay = result_publication::retained_recovery_audit_event(
        &mut after_store,
        "request-4",
        &source.public_key(),
        "Requester",
        &owner,
    )
    .unwrap()
    .expect("stored bytes remain publishable");
    assert_eq!(replay.0.id, event.id);
    for request_id in group_ids {
        after_store
            .requests
            .get_mut(&request_id)
            .unwrap()
            .audit_published = true;
    }
    assert!(result_publication::retained_recovery_audit_event(
        &mut after_store,
        "request-0",
        &source.public_key(),
        "Requester",
        &owner,
    )
    .unwrap()
    .is_none());
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
fn applying_result_can_be_backdated_below_a_same_second_terminal_result() {
    let owner = Keys::generate();
    let source = Keys::generate().public_key();
    let event = result_publication::build_management_result_event(
        &owner,
        &source,
        "b2f095a4-664b-4b41-a6fd-4460fa38e520",
        false,
        &serde_json::json!({"status": "applying"}),
        Some(nostr::Timestamp::from(123_u64)),
    )
    .unwrap();

    assert_eq!(event.created_at.as_secs(), 123);
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

    let create_secret: AgentManagementPublishCommand = serde_json::from_value(serde_json::json!({
        "action": "create_secret",
        "requestId": "b2f095a4-664b-4b41-a6fd-4460fa38e520",
        "sourcePubkey": "11".repeat(32),
        "agentPubkey": "22".repeat(32),
        "privateKeyNsec": "nsec1example"
    }))
    .unwrap();
    assert!(matches!(
        create_secret,
        AgentManagementPublishCommand::CreateSecret { .. }
    ));

    let applying: AgentManagementPublishCommand = serde_json::from_value(serde_json::json!({
        "action": "applying",
        "requestId": "b2f095a4-664b-4b41-a6fd-4460fa38e520",
        "sourcePubkey": "11".repeat(32)
    }))
    .unwrap();
    assert!(matches!(
        applying,
        AgentManagementPublishCommand::Applying { .. }
    ));
}
