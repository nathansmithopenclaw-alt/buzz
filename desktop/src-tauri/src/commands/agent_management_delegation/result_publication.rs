use super::*;

fn is_interrupted_recovery(entry: &RequestLedgerEntry) -> bool {
    entry.status == RequestStatus::Failed
        && entry.recovery_batch_id.is_some()
        && entry
            .result
            .as_ref()
            .and_then(|result| result.get("code"))
            .and_then(Value::as_str)
            == Some("interrupted_during_apply")
}

fn recovery_group_ids(
    ledger: &AgentManagementState,
    source_hex: &str,
    recovery_batch_id: &str,
    audit_channel_id: &str,
) -> Vec<String> {
    ledger
        .requests
        .iter()
        .filter(|(_, entry)| {
            entry.source_pubkey == source_hex
                && entry.recovery_batch_id.as_deref() == Some(recovery_batch_id)
                && entry.audit_channel_id.as_deref() == Some(audit_channel_id)
                && is_interrupted_recovery(entry)
        })
        .map(|(request_id, _)| request_id.clone())
        .collect()
}

pub(super) fn build_audit_receipt_event(
    owner_keys: &Keys,
    source_hex: &str,
    requester_name: &str,
    capability: DelegatedCapability,
    target: &str,
    succeeded: bool,
    owner_reviewed: bool,
    request_id: &str,
    channel: &str,
    env_summary: Option<&str>,
) -> Result<Event, String> {
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
        DelegatedCapability::GlobalConfig => "global agent config",
        DelegatedCapability::Teams => "teams",
    };
    let outcome = if succeeded { "applied" } else { "failed" };
    let authority = if owner_reviewed {
        "owner-reviewed"
    } else {
        "delegated"
    };
    let mut content = format!(
        "@{safe_name} {authority} {capability_label} {outcome}: {safe_target}\n\nRequest: `{request_id}`"
    );
    if let Some(summary) = env_summary {
        content.push_str("\nEnv fingerprints: ");
        content.push_str(summary);
    }
    let h = Tag::parse(["h", channel])
        .map_err(|error| format!("failed to build audit channel tag: {error}"))?;
    let p = Tag::parse(["p", source_hex])
        .map_err(|error| format!("failed to build audit mention tag: {error}"))?;
    EventBuilder::new(Kind::Custom(9), content)
        .tags([h, p])
        .sign_with_keys(owner_keys)
        .map_err(|error| format!("failed to sign audit receipt: {error}"))
}

fn build_recovery_audit_event(
    owner_keys: &Keys,
    source: &PublicKey,
    requester_name: &str,
    audit_channel_id: &str,
    request_ids: &[String],
) -> Result<Event, String> {
    let safe_name: String = requester_name
        .chars()
        .filter(|character| !character.is_control() && *character != '@')
        .take(120)
        .collect();
    let request_label = if request_ids.len() == 1 {
        "request"
    } else {
        "requests"
    };
    let ids = request_ids
        .iter()
        .map(|request_id| format!("`{request_id}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let content = format!(
        "@{safe_name} Desktop recovered {} interrupted delegated management {request_label}. Completion is unknown; no mutation was retried automatically.\n\nRequests: {ids}\n\nIf these changes are still wanted, inspect current state and submit only the intended changes.",
        request_ids.len()
    );
    let h = Tag::parse(["h", audit_channel_id])
        .map_err(|error| format!("failed to build recovery audit channel tag: {error}"))?;
    let source_hex = source.to_hex();
    let p = Tag::parse(["p", source_hex.as_str()])
        .map_err(|error| format!("failed to build recovery audit mention tag: {error}"))?;
    EventBuilder::new(Kind::Custom(9), content)
        .tags([h, p])
        .sign_with_keys(owner_keys)
        .map_err(|error| format!("failed to sign recovery audit receipt: {error}"))
}

pub(super) fn retained_recovery_audit_event(
    ledger: &mut AgentManagementState,
    request_id: &str,
    source: &PublicKey,
    requester_name: &str,
    owner_keys: &Keys,
) -> Result<Option<(Event, Vec<String>)>, String> {
    let source_hex = source.to_hex();
    let seed = ledger
        .requests
        .get(request_id)
        .ok_or("management request is not registered")?;
    if seed.source_pubkey != source_hex {
        return Err("recovery audit does not match the registered requester".into());
    }
    if !is_interrupted_recovery(seed) {
        return Ok(None);
    }
    let recovery_batch_id = seed.recovery_batch_id.clone().expect("checked above");
    let audit_channel_id = seed
        .audit_channel_id
        .clone()
        .ok_or("recovery request has no authorization audit channel")?;
    let group_ids = recovery_group_ids(ledger, &source_hex, &recovery_batch_id, &audit_channel_id);
    if group_ids
        .iter()
        .all(|request_id| ledger.requests[request_id].audit_published)
    {
        return Ok(None);
    }

    let stored_events = group_ids
        .iter()
        .filter_map(|request_id| ledger.requests[request_id].audit_event.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    if stored_events.len() > 1 {
        return Err("recovery audit group contains conflicting stored events".into());
    }
    let event = match stored_events.first() {
        Some(raw) => Event::from_json(*raw)
            .map_err(|error| format!("stored recovery audit receipt is invalid: {error}"))?,
        None => build_recovery_audit_event(
            owner_keys,
            source,
            requester_name,
            &audit_channel_id,
            &group_ids,
        )?,
    };
    if event.pubkey != owner_keys.public_key() {
        return Err("stored recovery audit receipt is not owner-signed".into());
    }
    let raw_event = event.as_json();
    for grouped_request_id in &group_ids {
        let entry = ledger
            .requests
            .get_mut(grouped_request_id)
            .expect("group ids came from the ledger");
        entry.audit_event = Some(raw_event.clone());
        entry.updated_at = now_iso();
    }
    Ok(Some((event, group_ids)))
}

pub(super) fn verify_recovery_publish_response(
    event: &Event,
    response: &crate::relay::SubmitEventResponse,
) -> Result<(), String> {
    let expected = event.id.to_hex();
    if response.event_id != expected {
        return Err(format!(
            "relay acknowledged event {} while publishing recovery audit {expected}",
            response.event_id
        ));
    }
    Ok(())
}

pub(super) async fn publish_agent_management_recovery_audit_receipt(
    request_id: &str,
    source: &PublicKey,
    requester_name: &str,
    owner_keys: &Keys,
    app: &AppHandle,
) -> Result<bool, String> {
    let (is_recovery, prepared) = {
        let state_handle = app.state::<AppState>();
        let _guard = state_handle
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut ledger = load_state(app)?;
        let entry = ledger
            .requests
            .get(request_id)
            .ok_or("management request is not registered")?;
        if entry.source_pubkey != source.to_hex() {
            return Err("recovery audit does not match the registered requester".into());
        }
        let is_recovery = is_interrupted_recovery(entry);
        let prepared = retained_recovery_audit_event(
            &mut ledger,
            request_id,
            source,
            requester_name,
            owner_keys,
        )?;
        if prepared.is_some() {
            // The atomically stored signed bytes are the claim. A crash before
            // this save leaves no claim; a crash after it reuses the bytes.
            save_state(app, &ledger)?;
        }
        (is_recovery, prepared)
    };
    if !is_recovery {
        return Ok(false);
    }
    let Some((event, group_ids)) = prepared else {
        return Ok(true);
    };
    let response = crate::relay::submit_signed_event_with_keys(
        &event,
        &app.state::<AppState>(),
        owner_keys,
        None,
    )
    .await?;
    verify_recovery_publish_response(&event, &response)?;

    let state_handle = app.state::<AppState>();
    let _guard = state_handle
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut ledger = load_state(app)?;
    let raw_event = event.as_json();
    for grouped_request_id in group_ids {
        let entry = ledger
            .requests
            .get_mut(&grouped_request_id)
            .ok_or("recovery request disappeared after audit publication")?;
        if entry.audit_event.as_deref() != Some(raw_event.as_str()) {
            return Err("recovery audit group changed during publication".into());
        }
        entry.audit_published = true;
        entry.updated_at = now_iso();
    }
    save_state(app, &ledger)?;
    Ok(true)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagementApplyResult<'a> {
    #[serde(rename = "type")]
    result_type: &'static str,
    request_id: &'a str,
    succeeded: bool,
    result: &'a Value,
}

pub(super) fn build_management_result_event(
    owner_keys: &Keys,
    source: &PublicKey,
    request_id: &str,
    succeeded: bool,
    result: &Value,
    created_at: Option<nostr::Timestamp>,
) -> Result<Event, String> {
    let content = encrypt_observer_payload(
        owner_keys,
        source,
        &ManagementApplyResult {
            result_type: "agent_management_apply_result",
            request_id,
            succeeded,
            result,
        },
    )
    .map_err(|error| format!("failed to encrypt management result: {error}"))?;
    let d_tag = format!("buzz-agent-management-result:{request_id}");
    let d = Tag::parse(["d", d_tag.as_str()])
        .map_err(|error| format!("failed to build result d tag: {error}"))?;
    let p = Tag::parse(["p", source.to_hex().as_str()])
        .map_err(|error| format!("failed to build result p tag: {error}"))?;
    let mut builder = EventBuilder::new(Kind::Custom(30_078), content).tags([d, p]);
    if let Some(created_at) = created_at {
        builder = builder.custom_created_at(created_at);
    }
    builder
        .sign_with_keys(owner_keys)
        .map_err(|error| format!("failed to sign management result: {error}"))
}

pub(super) async fn publish_agent_management_result(
    request_id: String,
    source_pubkey: String,
    result: Value,
    app: AppHandle,
) -> Result<(), String> {
    let request_id = canonical_request_id(&request_id)?;
    if value_contains_secret(&result) {
        return Err("management result must not contain secrets".into());
    }
    let source = PublicKey::parse(source_pubkey.trim())
        .map_err(|error| format!("invalid management requester pubkey: {error}"))?;
    let source_hex = source.to_hex();
    let owner_keys = app.state::<AppState>().signing_keys()?;
    let (succeeded, result_event) = {
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
        if !matches!(entry.status, RequestStatus::Applied | RequestStatus::Failed)
            || entry.source_pubkey != source_hex
            || entry.result.as_ref() != Some(&result)
        {
            return Err("management result does not match the applied request".into());
        }
        let succeeded = entry.status == RequestStatus::Applied;
        let event = match &entry.result_event {
            Some(raw) => Event::from_json(raw)
                .map_err(|error| format!("stored management result is invalid: {error}"))?,
            None => {
                let event = build_management_result_event(
                    &owner_keys,
                    &source,
                    &request_id,
                    succeeded,
                    &result,
                    None,
                )?;
                entry.result_event = Some(event.as_json());
                entry.updated_at = now_iso();
                save_state(&app, &ledger)?;
                event
            }
        };
        (succeeded, event)
    };
    crate::relay::submit_signed_event_with_keys(
        &result_event,
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
        .ok_or("management request disappeared after result publication")?;
    let expected_status = if succeeded {
        RequestStatus::Applied
    } else {
        RequestStatus::Failed
    };
    if entry.status == expected_status && entry.result.as_ref() == Some(&result) {
        entry.result_published = true;
        entry.updated_at = now_iso();
        save_state(&app, &ledger)?;
    }
    Ok(())
}

pub(super) async fn publish_agent_management_applying_result(
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
        if entry.status != RequestStatus::Applying || entry.source_pubkey != source_hex {
            return Err("management request is not applying".into());
        }
    }
    let owner_keys = app.state::<AppState>().signing_keys()?;
    let result = serde_json::json!({
        "code": "request_already_applying",
        "error": "Management request is already applying; retry later with the same request id",
        "retryable": true,
        "status": "applying"
    });
    let event = build_management_result_event(
        &owner_keys,
        &source,
        &request_id,
        false,
        &result,
        Some(nostr::Timestamp::from(
            nostr::Timestamp::now().as_secs().saturating_sub(1),
        )),
    )?;
    crate::relay::submit_signed_event_with_keys(&event, &app.state::<AppState>(), &owner_keys, None)
        .await
        .map(|_| ())
}
