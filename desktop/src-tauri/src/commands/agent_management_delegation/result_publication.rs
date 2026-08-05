use super::*;

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
