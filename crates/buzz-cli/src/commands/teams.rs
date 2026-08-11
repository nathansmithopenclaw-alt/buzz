use buzz_core::observer::{
    decrypt_observer_payload, encrypt_observer_payload, OBSERVER_FRAME_TELEMETRY,
};
use nostr::{Event, PublicKey};
use serde::Serialize;
use serde_json::Value;

use crate::{client::BuzzClient, error::CliError, TeamsCmd};

const REQUEST_TYPE: &str = "agent_team_management_request";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TeamRequest<T> {
    #[serde(rename = "type")]
    request_type: &'static str,
    action: &'static str,
    request_id: String,
    intent: &'static str,
    request: T,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ObserverEnvelope<T> {
    seq: u64,
    timestamp: String,
    kind: &'static str,
    agent_index: Option<usize>,
    channel_id: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    payload: TeamRequest<T>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateTeamRequest {
    channel_id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    members: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateTeamRequest {
    channel_id: String,
    team: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    members: Option<Vec<String>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AddTeamToChannelRequest {
    channel_id: String,
    team: String,
}

fn require_owner(client: &BuzzClient) -> Result<PublicKey, CliError> {
    let owner = client
        .auth_tag_owner_hex()
        .ok_or_else(|| CliError::Auth("delegated team commands require BUZZ_AUTH_TAG".into()))?;
    PublicKey::parse(&owner).map_err(|error| CliError::Auth(format!("invalid owner: {error}")))
}

fn required(value: String, label: &str, max: usize) -> Result<String, CliError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > max {
        return Err(CliError::Usage(format!(
            "{label} is required and must be at most {max} characters"
        )));
    }
    Ok(value.to_string())
}

fn optional(value: Option<String>, max: usize, label: &str) -> Result<Option<String>, CliError> {
    value
        .map(|value| {
            if value.chars().count() > max {
                Err(CliError::Usage(format!("{label} is too long (max {max})")))
            } else {
                Ok(value)
            }
        })
        .transpose()
}

fn build<T: Serialize>(
    client: &BuzzClient,
    owner: &PublicKey,
    channel_id: &str,
    action: &'static str,
    request_id: Option<String>,
    request: T,
) -> Result<(Event, String), CliError> {
    uuid::Uuid::parse_str(channel_id)
        .map_err(|_| CliError::Usage(format!("invalid channel UUID: {channel_id}")))?;
    let request_id = request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let request_id = uuid::Uuid::parse_str(&request_id)
        .map_err(|_| CliError::Usage(format!("invalid request UUID: {request_id}")))?
        .to_string();
    let payload = ObserverEnvelope {
        seq: 0,
        timestamp: chrono::Utc::now().to_rfc3339(),
        kind: REQUEST_TYPE,
        agent_index: None,
        channel_id: Some(channel_id.to_string()),
        session_id: None,
        turn_id: None,
        payload: TeamRequest {
            request_type: REQUEST_TYPE,
            action,
            request_id: request_id.clone(),
            intent: "apply",
            request,
        },
    };
    let encrypted = encrypt_observer_payload(client.keys(), owner, &payload)
        .map_err(|error| CliError::Other(format!("encrypt team request: {error}")))?;
    let event = buzz_sdk::build_agent_observer_frame(
        &owner.to_hex(),
        &client.keys().public_key().to_hex(),
        OBSERVER_FRAME_TELEMETRY,
        &encrypted,
    )
    .map_err(|error| CliError::Other(format!("build team request: {error}")))?
    .sign_with_keys(client.keys())
    .map_err(|error| CliError::Other(format!("sign team request: {error}")))?;
    Ok((event, request_id))
}

async fn publish_and_wait(
    client: &BuzzClient,
    owner: &PublicKey,
    event: Event,
    request_id: &str,
) -> Result<(Value, bool), CliError> {
    client.publish_ephemeral_event(event).await?;
    let d_tag = format!("buzz-agent-management-result:{request_id}");
    let filter = serde_json::json!({
        "kinds": [30_078],
        "authors": [owner.to_hex()],
        "#d": [d_tag],
        "#p": [client.keys().public_key().to_hex()],
        "limit": 1,
    });
    for _ in 0..120 {
        let raw = client.query(&filter).await?;
        let events: Vec<Event> = serde_json::from_str(&raw)
            .map_err(|error| CliError::Other(format!("invalid team result query: {error}")))?;
        if let Some(event) = events.into_iter().find(|event| {
            event.pubkey == *owner && event.verify().is_ok() && event.kind.as_u16() == 30_078
        }) {
            let value: Value =
                decrypt_observer_payload(client.keys(), &event).map_err(|error| {
                    CliError::Other(format!("invalid encrypted team result: {error}"))
                })?;
            if value["type"] == "agent_management_apply_result" && value["requestId"] == request_id
            {
                if value["succeeded"] == true {
                    return Ok((value["result"].clone(), true));
                }
                if value["result"]["queuedForReview"] == true {
                    return Ok((value["result"].clone(), false));
                }
                let message = value["result"]["error"]
                    .as_str()
                    .unwrap_or("Desktop rejected the delegated team operation");
                return Err(CliError::Other(message.to_string()));
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err(CliError::DeliveryUnknown(
        format!(
            "team request {request_id} reached the relay, but no Desktop apply result arrived within 60 seconds; retry with --request-id {request_id}"
        ),
    ))
}

pub async fn dispatch(command: TeamsCmd, client: &BuzzClient) -> Result<(), CliError> {
    let owner = require_owner(client)?;
    match command {
        TeamsCmd::Create {
            request_id,
            channel,
            name,
            description,
            instructions,
            agents,
        } => {
            let request = CreateTeamRequest {
                channel_id: channel.clone(),
                name: required(name, "team name", 120)?,
                description: optional(description, 2_000, "description")?,
                instructions: optional(instructions, 20_000, "instructions")?,
                members: agents
                    .into_iter()
                    .map(|member| required(member, "agent", 200))
                    .collect::<Result<_, _>>()?,
            };
            let (event, request_id) =
                build(client, &owner, &channel, "create", request_id, request)?;
            let (result, applied) = publish_and_wait(client, &owner, event, &request_id).await?;
            println!(
                "{}",
                serde_json::json!({"request_id": request_id, "applied": applied, "queued_for_review": !applied, "result": result})
            );
            Ok(())
        }
        TeamsCmd::Update {
            request_id,
            channel,
            team,
            name,
            description,
            instructions,
            agents,
            replace_members,
        } => {
            let request = UpdateTeamRequest {
                channel_id: channel.clone(),
                team: required(team, "team", 200)?,
                name: optional(name, 120, "team name")?,
                description: optional(description, 2_000, "description")?,
                instructions: optional(instructions, 20_000, "instructions")?,
                members: replace_members.then_some(
                    agents
                        .into_iter()
                        .map(|member| required(member, "agent", 200))
                        .collect::<Result<_, _>>()?,
                ),
            };
            let (event, request_id) =
                build(client, &owner, &channel, "update", request_id, request)?;
            let (result, applied) = publish_and_wait(client, &owner, event, &request_id).await?;
            println!(
                "{}",
                serde_json::json!({"request_id": request_id, "applied": applied, "queued_for_review": !applied, "result": result})
            );
            Ok(())
        }
        TeamsCmd::AddToChannel {
            request_id,
            channel,
            team,
        } => {
            let request = AddTeamToChannelRequest {
                channel_id: channel.clone(),
                team: required(team, "team", 200)?,
            };
            let (event, request_id) = build(
                client,
                &owner,
                &channel,
                "add_to_channel",
                request_id,
                request,
            )?;
            let (result, applied) = publish_and_wait(client, &owner, event, &request_id).await?;
            println!(
                "{}",
                serde_json::json!({"request_id": request_id, "applied": applied, "queued_for_review": !applied, "result": result})
            );
            Ok(())
        }
        TeamsCmd::List => {
            let raw = client
                .query(&serde_json::json!({
                    "kinds": [30_176],
                    "authors": [owner.to_hex()],
                    "limit": 500,
                }))
                .await?;
            let events: Vec<Event> = serde_json::from_str(&raw)
                .map_err(|error| CliError::Other(format!("invalid teams query: {error}")))?;
            let teams: Vec<Value> = events
                .into_iter()
                .filter(|event| {
                    event.pubkey == owner && event.kind.as_u16() == 30_176 && event.verify().is_ok()
                })
                .filter_map(|event| {
                    let id = event.tags.iter().find_map(|tag| {
                        let parts = tag.as_slice();
                        (parts.first().map(String::as_str) == Some("d"))
                            .then(|| parts.get(1).cloned())
                            .flatten()
                    })?;
                    let mut content: Value = serde_json::from_str(&event.content).ok()?;
                    content.as_object_mut()?.insert("id".into(), id.into());
                    Some(content)
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string(&teams)
                    .map_err(|error| CliError::Other(error.to_string()))?
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_omits_absent_optional_fields() {
        let value = serde_json::to_value(CreateTeamRequest {
            channel_id: "7c07e659-3610-42f4-9a5e-1e9973c09da9".into(),
            name: "Review team".into(),
            description: None,
            instructions: None,
            members: Vec::new(),
        })
        .unwrap();

        assert!(value.get("description").is_none());
        assert!(value.get("instructions").is_none());
        assert_eq!(value["members"], serde_json::json!([]));
    }

    #[test]
    fn update_omits_all_absent_optional_fields() {
        let value = serde_json::to_value(UpdateTeamRequest {
            channel_id: "7c07e659-3610-42f4-9a5e-1e9973c09da9".into(),
            team: "Review team".into(),
            name: None,
            description: None,
            instructions: None,
            members: None,
        })
        .unwrap();

        for field in ["name", "description", "instructions", "members"] {
            assert!(value.get(field).is_none(), "{field} serialized as null");
        }
    }
}
