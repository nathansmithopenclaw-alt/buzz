use super::*;

fn provider_record(deployed: bool) -> ManagedAgentRecord {
    let mut record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
        "pubkey": "agent", "name": "Agent", "relay_url": "", "acp_command": "",
        "agent_command": "", "agent_args": [], "mcp_command": "",
        "turn_timeout_seconds": 0, "system_prompt": null, "created_at": "",
        "updated_at": "", "last_started_at": null, "last_stopped_at": null,
        "last_exit_code": null, "last_error": null
    }))
    .unwrap();
    record.backend = crate::managed_agents::BackendKind::Provider {
        id: "provider".into(),
        config: serde_json::json!({}),
    };
    record.backend_agent_id = deployed.then(|| "deployment".to_string());
    record
}

#[test]
fn deployed_provider_rejects_access_edits_that_cannot_be_revoked() {
    let error = ensure_access_policy_change_supported(&provider_record(true), true)
        .expect_err("deployed provider access edit must fail closed");
    assert!(error.contains("no explicit stop or revocation acknowledgement"));
}

#[test]
fn undeployed_provider_accepts_access_edits() {
    ensure_access_policy_change_supported(&provider_record(false), true)
        .expect("no running provider deployment can retain stale access");
}

#[test]
fn effort_update_distinguishes_omitted_clear_and_set() {
    for (json, expected) in [
        (serde_json::json!({"pubkey":"agent"}), None),
        (
            serde_json::json!({"pubkey":"agent","effortLevel":null}),
            Some(None),
        ),
        (
            serde_json::json!({"pubkey":"agent","effortLevel":"high"}),
            Some(Some("high".to_string())),
        ),
    ] {
        let input: UpdateManagedAgentRequest = serde_json::from_value(json).unwrap();
        assert_eq!(input.effort_level, expected);
        let mut record = provider_record(false);
        record.backend = crate::managed_agents::BackendKind::Local;
        record.effort_level = Some("low".into());
        record
            .env_vars
            .insert("BUZZ_ACP_THINKING_EFFORT".into(), "low".into());
        record
            .env_vars
            .insert("UNRELATED".into(), "preserve".into());
        validate_effort_update(&record, input.effort_level.as_ref()).unwrap();
        apply_effort_update(&mut record, input.effort_level);
        assert_eq!(
            record.effort_level,
            expected.clone().unwrap_or(Some("low".into()))
        );
        assert_eq!(
            record.env_vars.contains_key("BUZZ_ACP_THINKING_EFFORT"),
            expected.is_none()
        );
        assert_eq!(
            record.env_vars.get("UNRELATED").map(String::as_str),
            Some("preserve")
        );
    }
}

#[test]
fn remote_effort_edits_reject_even_clear_and_leave_record_untouched() {
    for value in [None, Some("high".to_string())] {
        let record = provider_record(true);
        assert!(validate_effort_update(&record, Some(&value)).is_err());
        assert!(record.effort_level.is_none());
    }
}
