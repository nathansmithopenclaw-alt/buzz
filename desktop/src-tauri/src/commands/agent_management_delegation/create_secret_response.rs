use buzz_core_pkg::observer::encrypt_observer_payload;
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, PublicKey, Tag};
use serde::Serialize;
use serde_json::Value;

use super::{AgentManagementState, DelegatedCapability, RequestStatus};
use crate::util::now_iso;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentCreateSecretResponse<'a> {
    #[serde(rename = "type")]
    response_type: &'static str,
    request_id: &'a str,
    pubkey: &'a str,
    nsec: &'a str,
}

pub(super) fn retained_agent_create_secret_event(
    ledger: &mut AgentManagementState,
    request_id: &str,
    source: &PublicKey,
    agent_keys: &Keys,
    private_key_nsec: &str,
    owner_keys: &Keys,
) -> Result<(Event, bool), String> {
    let source_hex = source.to_hex();
    let agent_hex = agent_keys.public_key().to_hex();
    let entry = ledger
        .requests
        .get_mut(request_id)
        .ok_or("management request is not registered")?;
    let recorded_pubkey = entry
        .result
        .as_ref()
        .and_then(|value| value.get("pubkey"))
        .and_then(Value::as_str);
    if entry.status != RequestStatus::Applied
        || entry.source_pubkey != source_hex
        || entry.capability != DelegatedCapability::AgentCreation
        || recorded_pubkey != Some(agent_hex.as_str())
    {
        return Err("create secret response does not match the applied request".into());
    }
    if let Some(raw) = &entry.secret_response_event {
        let event = Event::from_json(raw)
            .map_err(|error| format!("stored create-secret response is invalid: {error}"))?;
        return Ok((event, false));
    }

    let response = AgentCreateSecretResponse {
        response_type: "agent_create_secret_response",
        request_id,
        pubkey: &agent_hex,
        nsec: private_key_nsec,
    };
    let encrypted = encrypt_observer_payload(owner_keys, source, &response)
        .map_err(|error| format!("failed to encrypt create result: {error}"))?;
    let d_tag = format!("buzz-agent-create-result:{request_id}");
    let d = Tag::parse(["d", d_tag.as_str()])
        .map_err(|error| format!("failed to build create-result d tag: {error}"))?;
    let p = Tag::parse(["p", source_hex.as_str()])
        .map_err(|error| format!("failed to build create-result p tag: {error}"))?;
    let event = EventBuilder::new(Kind::Custom(30_078), encrypted)
        .tags([d, p])
        .sign_with_keys(owner_keys)
        .map_err(|error| format!("failed to sign create result: {error}"))?;
    entry.secret_response_event = Some(event.as_json());
    entry.updated_at = now_iso();
    Ok((event, true))
}
