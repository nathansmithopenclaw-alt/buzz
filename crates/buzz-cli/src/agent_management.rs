//! Owner-reviewed agent draft requests published through Buzz observer frames.

use buzz_core::observer::{encrypt_observer_payload, OBSERVER_FRAME_TELEMETRY};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, PublicKey, Tag};
use serde::Serialize;

use crate::error::CliError;

const REQUEST_KIND: &str = "agent_management_request";
const MAX_NAME_CHARS: usize = 120;
const MAX_NAME_POOL_ENTRIES: usize = 100;
const MAX_PROMPT_CHARS: usize = 20_000;
const AVATAR_APPLY_REQUEST_KIND: &str = "agent_avatar_apply_request";
const AVATAR_AUTHORIZATION_KIND: u16 = 30_078;
const AVATAR_AUTHORIZATION_D_TAG: &str = "buzz-agent-avatar-apply";
const MAX_AVATAR_URL_CHARS: usize = 4_096;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentDraft {
    pub channel_id: String,
    pub display_name: String,
    pub system_prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub respond_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallelism: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAgentDraft {
    pub channel_id: String,
    pub agent_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub respond_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallelism: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_pool: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidatedUpdateAgentDraft {
    channel_id: String,
    agent_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    respond_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parallelism: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name_pool: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagementRequest<T> {
    #[serde(rename = "type")]
    request_type: &'static str,
    action: &'static str,
    request_id: String,
    intent: ManagementIntent,
    request: T,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagementIntent {
    Draft,
    Apply,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ObserverEvent<T> {
    seq: u64,
    timestamp: String,
    kind: &'static str,
    agent_index: Option<usize>,
    channel_id: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    payload: ManagementRequest<T>,
}

#[derive(Debug)]
pub struct BuiltDraftRequest {
    pub event: Event,
    pub request_id: String,
    pub action: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AvatarAuthorization {
    #[serde(rename = "type")]
    request_type: &'static str,
    request_id: String,
    target_pubkey: String,
    url: String,
    issued_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AvatarApplyRequest {
    #[serde(rename = "type")]
    request_type: &'static str,
    request_id: String,
    authorization_event: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AvatarObserverEvent {
    seq: u64,
    timestamp: String,
    kind: &'static str,
    agent_index: Option<usize>,
    channel_id: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    payload: AvatarApplyRequest,
}

#[derive(Debug)]
pub struct BuiltAvatarApplyRequest {
    pub event: Event,
    pub request_id: String,
    pub target_pubkey: String,
    pub url: String,
}

fn validate_avatar_url(value: String) -> Result<String, CliError> {
    let value = required(value, "avatar URL", MAX_AVATAR_URL_CHARS)?;
    let parsed = url::Url::parse(&value)
        .map_err(|error| CliError::Usage(format!("invalid avatar URL: {error}")))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(CliError::Usage(
            "avatar URL must be an absolute HTTPS URL".into(),
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(CliError::Usage(
            "avatar URL must not contain credentials".into(),
        ));
    }
    Ok(value)
}

/// Build a narrow avatar apply request. The invoking agent signs the observer
/// frame so it follows the existing authenticated owner-scoped route; the
/// embedded authorization event separately authorizes the requested target.
pub fn build_set_avatar(
    owner_keys: &Keys,
    requester_keys: &Keys,
    target: &PublicKey,
    url: String,
) -> Result<BuiltAvatarApplyRequest, CliError> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let issued_at = chrono::Utc::now().to_rfc3339();
    let target_pubkey = target.to_hex();
    let url = validate_avatar_url(url)?;
    let authorization = AvatarAuthorization {
        request_type: AVATAR_APPLY_REQUEST_KIND,
        request_id: request_id.clone(),
        target_pubkey: target_pubkey.clone(),
        url: url.clone(),
        issued_at: issued_at.clone(),
    };
    let content = serde_json::to_string(&authorization)
        .map_err(|error| CliError::Other(format!("serialize avatar authorization: {error}")))?;
    let d_tag = Tag::parse(["d", AVATAR_AUTHORIZATION_D_TAG])
        .map_err(|error| CliError::Other(format!("build avatar authorization tag: {error}")))?;
    let authorization_event = EventBuilder::new(Kind::Custom(AVATAR_AUTHORIZATION_KIND), content)
        .tags([d_tag])
        .sign_with_keys(owner_keys)
        .map_err(|error| CliError::Other(format!("sign avatar authorization: {error}")))?;
    let payload = AvatarObserverEvent {
        seq: 0,
        timestamp: issued_at,
        kind: AVATAR_APPLY_REQUEST_KIND,
        agent_index: None,
        channel_id: None,
        session_id: None,
        turn_id: None,
        payload: AvatarApplyRequest {
            request_type: AVATAR_APPLY_REQUEST_KIND,
            request_id: request_id.clone(),
            authorization_event: authorization_event.as_json(),
        },
    };
    let requester_pubkey = requester_keys.public_key().to_hex();
    let encrypted = encrypt_observer_payload(requester_keys, &owner_keys.public_key(), &payload)
        .map_err(|error| CliError::Other(format!("encrypt avatar request: {error}")))?;
    let event = buzz_sdk::build_agent_observer_frame(
        &owner_keys.public_key().to_hex(),
        &requester_pubkey,
        OBSERVER_FRAME_TELEMETRY,
        &encrypted,
    )
    .map_err(|error| CliError::Other(format!("build avatar request: {error}")))?
    .sign_with_keys(requester_keys)
    .map_err(|error| CliError::Other(format!("sign avatar request: {error}")))?;
    Ok(BuiltAvatarApplyRequest {
        event,
        request_id,
        target_pubkey,
        url,
    })
}

fn required(value: String, label: &str, max: usize) -> Result<String, CliError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(CliError::Usage(format!("{label} is required")));
    }
    if value.chars().count() > max {
        return Err(CliError::Usage(format!(
            "{label} is too long (max {max} characters)"
        )));
    }
    Ok(value.to_owned())
}

fn optional(value: Option<String>, label: &str) -> Result<Option<String>, CliError> {
    value.map(|value| required(value, label, 300)).transpose()
}

fn optional_name_pool(value: Option<String>) -> Result<Option<Vec<String>>, CliError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let names = value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| required(name.to_owned(), "instance name", MAX_NAME_CHARS))
        .collect::<Result<Vec<_>, _>>()?;
    if names.len() > MAX_NAME_POOL_ENTRIES {
        return Err(CliError::Usage(format!(
            "instance-name-pool has too many names (max {MAX_NAME_POOL_ENTRIES})"
        )));
    }
    Ok(Some(names))
}

fn build<T: Serialize>(
    keys: &Keys,
    owner: &PublicKey,
    channel_id: String,
    action: &'static str,
    intent: ManagementIntent,
    request_id: Option<String>,
    request: T,
) -> Result<BuiltDraftRequest, CliError> {
    let request_id = request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let request_id = uuid::Uuid::parse_str(&request_id)
        .map_err(|_| CliError::Usage(format!("invalid request UUID: {request_id}")))?
        .to_string();
    let payload = ObserverEvent {
        seq: 0,
        timestamp: chrono::Utc::now().to_rfc3339(),
        kind: REQUEST_KIND,
        agent_index: None,
        channel_id: Some(channel_id),
        session_id: None,
        turn_id: None,
        payload: ManagementRequest {
            request_type: REQUEST_KIND,
            action,
            request_id: request_id.clone(),
            intent,
            request,
        },
    };
    let encrypted = encrypt_observer_payload(keys, owner, &payload)
        .map_err(|error| CliError::Other(format!("could not encrypt draft request: {error}")))?;
    let event = buzz_sdk::build_agent_observer_frame(
        &owner.to_hex(),
        &keys.public_key().to_hex(),
        OBSERVER_FRAME_TELEMETRY,
        &encrypted,
    )
    .map_err(|error| CliError::Other(format!("could not build draft request: {error}")))?
    .sign_with_keys(keys)
    .map_err(|error| CliError::Other(format!("could not sign draft request: {error}")))?;
    Ok(BuiltDraftRequest {
        event,
        request_id,
        action,
    })
}

pub fn build_create(
    keys: &Keys,
    owner: &PublicKey,
    draft: CreateAgentDraft,
) -> Result<BuiltDraftRequest, CliError> {
    build_create_with_intent(keys, owner, draft, ManagementIntent::Draft, None)
}

pub fn build_apply_create(
    keys: &Keys,
    owner: &PublicKey,
    draft: CreateAgentDraft,
    request_id: Option<String>,
) -> Result<BuiltDraftRequest, CliError> {
    build_create_with_intent(keys, owner, draft, ManagementIntent::Apply, request_id)
}

fn build_create_with_intent(
    keys: &Keys,
    owner: &PublicKey,
    draft: CreateAgentDraft,
    intent: ManagementIntent,
    request_id: Option<String>,
) -> Result<BuiltDraftRequest, CliError> {
    let channel_id = required(draft.channel_id, "channel", 128)?;
    uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| CliError::Usage(format!("invalid channel UUID: {channel_id}")))?;
    let respond_to = optional(draft.respond_to, "respond-to")?;
    if respond_to
        .as_deref()
        .is_some_and(|value| value != "owner-only" && value != "anyone")
    {
        return Err(CliError::Usage(
            "respond-to must be owner-only or anyone".into(),
        ));
    }
    if draft
        .parallelism
        .is_some_and(|parallelism| !(1..=32).contains(&parallelism))
    {
        return Err(CliError::Usage(
            "parallelism must be between 1 and 32".into(),
        ));
    }
    let request = CreateAgentDraft {
        channel_id: channel_id.clone(),
        display_name: required(draft.display_name, "display name", MAX_NAME_CHARS)?,
        system_prompt: required(draft.system_prompt, "system prompt", MAX_PROMPT_CHARS)?,
        runtime: optional(draft.runtime, "runtime")?,
        provider: optional(draft.provider, "provider")?,
        model: optional(draft.model, "model")?,
        respond_to,
        parallelism: draft.parallelism,
    };
    build(
        keys, owner, channel_id, "create", intent, request_id, request,
    )
}

pub fn build_update(
    keys: &Keys,
    owner: &PublicKey,
    draft: UpdateAgentDraft,
) -> Result<BuiltDraftRequest, CliError> {
    let channel_id = required(draft.channel_id, "channel", 128)?;
    uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| CliError::Usage(format!("invalid channel UUID: {channel_id}")))?;
    let respond_to = optional(draft.respond_to, "respond-to")?;
    if respond_to
        .as_deref()
        .is_some_and(|value| value != "owner-only" && value != "anyone")
    {
        return Err(CliError::Usage(
            "respond-to must be owner-only or anyone".into(),
        ));
    }
    if draft
        .parallelism
        .is_some_and(|parallelism| !(1..=32).contains(&parallelism))
    {
        return Err(CliError::Usage(
            "parallelism must be between 1 and 32".into(),
        ));
    }
    let request = ValidatedUpdateAgentDraft {
        channel_id: channel_id.clone(),
        agent_name: required(draft.agent_name, "agent name", MAX_NAME_CHARS)?,
        display_name: optional(draft.display_name, "display name")?,
        system_prompt: draft
            .system_prompt
            .map(|value| required(value, "system prompt", MAX_PROMPT_CHARS))
            .transpose()?,
        runtime: optional(draft.runtime, "runtime")?,
        provider: optional(draft.provider, "provider")?,
        model: optional(draft.model, "model")?,
        respond_to,
        parallelism: draft.parallelism,
        name_pool: optional_name_pool(draft.name_pool)?,
    };
    if request.display_name.is_none()
        && request.system_prompt.is_none()
        && request.runtime.is_none()
        && request.provider.is_none()
        && request.model.is_none()
        && request.respond_to.is_none()
        && request.parallelism.is_none()
        && request.name_pool.is_none()
    {
        return Err(CliError::Usage(
            "include at least one field to update".into(),
        ));
    }
    build(
        keys,
        owner,
        channel_id,
        "update",
        ManagementIntent::Draft,
        None,
        request,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::observer::{decrypt_observer_payload, OBSERVER_AGENT_TAG, OBSERVER_FRAME_TAG};

    const CHANNEL: &str = "7c07e659-3610-42f4-9a5e-1e9973c09da9";

    #[test]
    fn set_avatar_is_requester_framed_and_owner_authorized_for_target() {
        let owner = Keys::generate();
        let requester = Keys::generate();
        let target = Keys::generate();
        let built = build_set_avatar(
            &owner,
            &requester,
            &target.public_key(),
            "https://relay.example/media/bolt.png".into(),
        )
        .unwrap();

        assert_eq!(built.event.kind.as_u16(), 24_200);
        assert_eq!(built.event.pubkey, requester.public_key());
        let decrypted: serde_json::Value = decrypt_observer_payload(&owner, &built.event).unwrap();
        let authorization =
            Event::from_json(decrypted["payload"]["authorizationEvent"].as_str().unwrap()).unwrap();
        assert!(authorization.verify().is_ok());
        assert_eq!(authorization.pubkey, owner.public_key());
        assert_eq!(authorization.kind, Kind::Custom(AVATAR_AUTHORIZATION_KIND));
        let content: serde_json::Value = serde_json::from_str(&authorization.content).unwrap();
        assert_eq!(content["targetPubkey"], target.public_key().to_hex());
        assert_eq!(content["url"], "https://relay.example/media/bolt.png");
    }

    #[test]
    fn set_avatar_rejects_non_https_or_credentialed_urls() {
        let owner = Keys::generate();
        let requester = Keys::generate();
        let target = Keys::generate();
        for url in [
            "http://relay.example/a.png",
            "https://user:pass@relay.example/a.png",
            "not-a-url",
        ] {
            assert!(
                build_set_avatar(&owner, &requester, &target.public_key(), url.into()).is_err()
            );
        }
    }

    #[test]
    fn create_is_owner_encrypted_and_matches_desktop_contract() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let built = build_create(
            &agent,
            &owner.public_key(),
            CreateAgentDraft {
                channel_id: CHANNEL.into(),
                display_name: "Research helper".into(),
                system_prompt: "Find sources.".into(),
                runtime: None,
                provider: None,
                model: None,
                respond_to: None,
                parallelism: None,
            },
        )
        .unwrap();

        assert_eq!(built.event.kind.as_u16(), 24_200);
        let tags: Vec<Vec<String>> = built
            .event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();
        assert!(tags
            .iter()
            .any(|tag| tag == &["p", &owner.public_key().to_hex()]));
        assert!(tags
            .iter()
            .any(|tag| tag == &[OBSERVER_AGENT_TAG, &agent.public_key().to_hex()]));
        assert!(tags
            .iter()
            .any(|tag| tag == &[OBSERVER_FRAME_TAG, OBSERVER_FRAME_TELEMETRY]));
        assert!(!tags
            .iter()
            .any(|tag| tag.first().map(String::as_str) == Some("h")));

        let payload: serde_json::Value = decrypt_observer_payload(&owner, &built.event).unwrap();
        assert_eq!(payload["kind"], REQUEST_KIND);
        assert_eq!(payload["channelId"], CHANNEL);
        assert_eq!(payload["payload"]["type"], REQUEST_KIND);
        assert_eq!(payload["payload"]["action"], "create");
        assert_eq!(payload["payload"]["intent"], "draft");
        assert_eq!(
            payload["payload"]["request"]["displayName"],
            "Research helper"
        );
        assert!(payload["payload"]["request"].get("runtime").is_none());
        assert!(payload["payload"]["request"].get("provider").is_none());
        assert!(payload["payload"]["request"].get("model").is_none());
        assert!(payload["payload"]["request"].get("respondTo").is_none());
        assert!(payload["payload"]["request"].get("parallelism").is_none());
    }

    #[test]
    fn apply_create_preserves_caller_request_uuid_and_apply_intent() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let request_id = "70fc630c-a0ca-42e8-9392-326c5d4ef9c1";
        let built = build_apply_create(
            &agent,
            &owner.public_key(),
            CreateAgentDraft {
                channel_id: CHANNEL.into(),
                display_name: "Research helper".into(),
                system_prompt: "Find sources.".into(),
                runtime: None,
                provider: None,
                model: None,
                respond_to: None,
                parallelism: None,
            },
            Some(request_id.into()),
        )
        .unwrap();

        assert_eq!(built.request_id, request_id);
        let payload: serde_json::Value = decrypt_observer_payload(&owner, &built.event).unwrap();
        assert_eq!(payload["payload"]["requestId"], request_id);
        assert_eq!(payload["payload"]["intent"], "apply");
        let request = &payload["payload"]["request"];
        for field in ["runtime", "provider", "model", "respondTo", "parallelism"] {
            assert!(request.get(field).is_none(), "{field} serialized as null");
        }
    }

    #[test]
    fn create_serializes_full_non_secret_configuration() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let built = build_create(
            &agent,
            &owner.public_key(),
            CreateAgentDraft {
                channel_id: CHANNEL.into(),
                display_name: "Research helper".into(),
                system_prompt: "Find sources.".into(),
                runtime: Some("codex".into()),
                provider: Some("openai".into()),
                model: Some("gpt-5".into()),
                respond_to: Some("anyone".into()),
                parallelism: Some(4),
            },
        )
        .unwrap();

        let payload: serde_json::Value = decrypt_observer_payload(&owner, &built.event).unwrap();
        let request = &payload["payload"]["request"];
        assert_eq!(request["runtime"], "codex");
        assert_eq!(request["provider"], "openai");
        assert_eq!(request["model"], "gpt-5");
        assert_eq!(request["respondTo"], "anyone");
        assert_eq!(request["parallelism"], 4);
        assert!(request.get("envVars").is_none());
    }

    #[test]
    fn update_requires_a_change() {
        let error = build_update(
            &Keys::generate(),
            &Keys::generate().public_key(),
            UpdateAgentDraft {
                channel_id: CHANNEL.into(),
                agent_name: "Scout".into(),
                display_name: None,
                system_prompt: None,
                runtime: None,
                provider: None,
                model: None,
                respond_to: None,
                parallelism: None,
                name_pool: None,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("at least one field"));
    }

    #[test]
    fn update_serializes_parallelism_and_instance_name_pool() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let built = build_update(
            &agent,
            &owner.public_key(),
            UpdateAgentDraft {
                channel_id: CHANNEL.into(),
                agent_name: "Scout".into(),
                display_name: None,
                system_prompt: None,
                runtime: None,
                provider: None,
                model: None,
                respond_to: None,
                parallelism: Some(6),
                name_pool: Some("Alpha, Beta".into()),
            },
        )
        .unwrap();

        let payload: serde_json::Value = decrypt_observer_payload(&owner, &built.event).unwrap();
        let request = &payload["payload"]["request"];
        assert_eq!(request["parallelism"], 6);
        assert_eq!(request["namePool"], serde_json::json!(["Alpha", "Beta"]));
        assert!(request.get("envVars").is_none());
    }

    #[test]
    fn update_can_clear_instance_name_pool() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let built = build_update(
            &agent,
            &owner.public_key(),
            UpdateAgentDraft {
                channel_id: CHANNEL.into(),
                agent_name: "Scout".into(),
                display_name: None,
                system_prompt: None,
                runtime: None,
                provider: None,
                model: None,
                respond_to: None,
                parallelism: None,
                name_pool: Some(String::new()),
            },
        )
        .unwrap();

        let payload: serde_json::Value = decrypt_observer_payload(&owner, &built.event).unwrap();
        assert_eq!(
            payload["payload"]["request"]["namePool"],
            serde_json::json!([])
        );
    }

    #[test]
    fn create_rejects_invalid_channel() {
        let error = build_create(
            &Keys::generate(),
            &Keys::generate().public_key(),
            CreateAgentDraft {
                channel_id: "general".into(),
                display_name: "Scout".into(),
                system_prompt: "Help".into(),
                runtime: None,
                provider: None,
                model: None,
                respond_to: None,
                parallelism: None,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("invalid channel UUID"));
    }
}
