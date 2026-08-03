use chrono::{DateTime, Duration, Utc};
use nostr::{Event, EventBuilder, JsonUtil, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppState,
    managed_agents::{
        load_managed_agents, load_personas, save_agent_definitions_and_instances,
        try_regenerate_nest, AgentDefinition, ManagedAgentRecord,
    },
    util::now_iso,
};

use super::{reconcile_agent_profile, ProfileReconcileData};

const AVATAR_APPLY_REQUEST_KIND: &str = "agent_avatar_apply_request";
const AVATAR_AUTHORIZATION_KIND: u16 = 30_078;
const AVATAR_AUTHORIZATION_D_TAG: &str = "buzz-agent-avatar-apply";
const AVATAR_RECEIPT_D_TAG_PREFIX: &str = "buzz-agent-avatar-apply-receipt:";
const MAX_AVATAR_URL_CHARS: usize = 4_096;
const MAX_AUTHORIZATION_AGE: Duration = Duration::minutes(5);
const MAX_FUTURE_SKEW: Duration = Duration::seconds(30);

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AvatarAuthorization {
    #[serde(rename = "type")]
    request_type: String,
    request_id: String,
    target_pubkey: String,
    url: String,
    issued_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyManagedAgentAvatarResult {
    applied: bool,
    target_pubkey: String,
    persona_id: String,
    instance_count: usize,
    reconciled_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AvatarApplyReceipt<'a> {
    #[serde(rename = "type")]
    receipt_type: &'static str,
    request_id: &'a str,
    target_pubkey: &'a str,
    url: &'a str,
    applied: bool,
    persona_id: &'a str,
    instance_count: usize,
    reconciled_count: usize,
}

struct ReconcileTarget {
    pubkey: String,
    data: ProfileReconcileData,
}

fn validate_avatar_url(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > MAX_AVATAR_URL_CHARS {
        return Err(format!(
            "avatar URL is required and must be at most {MAX_AVATAR_URL_CHARS} characters"
        ));
    }
    let parsed = url::Url::parse(value).map_err(|error| format!("invalid avatar URL: {error}"))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err("avatar URL must be an absolute HTTPS URL".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("avatar URL must not contain credentials".into());
    }
    Ok(value.to_string())
}

fn validate_avatar_authorization(
    raw_event: &str,
    owner: &PublicKey,
    source_pubkey: &str,
    now: DateTime<Utc>,
) -> Result<(AvatarAuthorization, DateTime<Utc>), String> {
    let event = Event::from_json(raw_event)
        .map_err(|error| format!("invalid avatar authorization event: {error}"))?;
    if event.verify().is_err()
        || event.pubkey != *owner
        || event.kind != Kind::Custom(AVATAR_AUTHORIZATION_KIND)
    {
        return Err("avatar authorization is not signed by this Desktop owner".into());
    }
    let has_expected_d_tag = event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.first().map(String::as_str) == Some("d")
            && parts.get(1).map(String::as_str) == Some(AVATAR_AUTHORIZATION_D_TAG)
    });
    if !has_expected_d_tag {
        return Err("avatar authorization has the wrong command tag".into());
    }

    let mut authorization: AvatarAuthorization = serde_json::from_str(&event.content)
        .map_err(|error| format!("invalid avatar authorization content: {error}"))?;
    if authorization.request_type != AVATAR_APPLY_REQUEST_KIND {
        return Err("avatar authorization has the wrong request type".into());
    }
    uuid::Uuid::parse_str(&authorization.request_id)
        .map_err(|_| "avatar authorization has an invalid request ID".to_string())?;
    let target = PublicKey::parse(authorization.target_pubkey.trim())
        .map_err(|error| format!("invalid avatar target pubkey: {error}"))?;
    PublicKey::parse(source_pubkey.trim())
        .map_err(|error| format!("invalid avatar source pubkey: {error}"))?;
    authorization.target_pubkey = target.to_hex();
    authorization.url = validate_avatar_url(&authorization.url)?;

    let issued_at = DateTime::parse_from_rfc3339(&authorization.issued_at)
        .map_err(|error| format!("invalid avatar authorization timestamp: {error}"))?
        .with_timezone(&Utc);
    if issued_at < now - MAX_AUTHORIZATION_AGE || issued_at > now + MAX_FUTURE_SKEW {
        return Err("avatar authorization is expired or too far in the future".into());
    }
    let event_time = DateTime::from_timestamp(event.created_at.as_secs() as i64, 0)
        .ok_or("avatar authorization event timestamp is invalid")?;
    if (issued_at - event_time).abs() > Duration::seconds(2) {
        return Err("avatar authorization timestamps do not agree".into());
    }
    Ok((authorization, issued_at))
}

fn apply_avatar_to_state(
    personas: &mut [AgentDefinition],
    records: &mut [ManagedAgentRecord],
    authorization: &AvatarAuthorization,
    issued_at: DateTime<Utc>,
) -> Result<(String, Vec<usize>, bool), String> {
    let target = records
        .iter()
        .find(|record| {
            record
                .pubkey
                .eq_ignore_ascii_case(&authorization.target_pubkey)
        })
        .ok_or_else(|| format!("managed agent not found: {}", authorization.target_pubkey))?;
    let persona_id = target
        .persona_id
        .clone()
        .ok_or_else(|| "managed agent has no persona definition".to_string())?;
    let persona = personas
        .iter_mut()
        .find(|persona| persona.id == persona_id)
        .ok_or_else(|| format!("persona definition not found: {persona_id}"))?;

    let prior_update = DateTime::parse_from_rfc3339(&persona.updated_at)
        .ok()
        .map(|value| value.with_timezone(&Utc));
    if prior_update.is_some_and(|updated_at| updated_at >= issued_at) {
        return Ok((persona_id, Vec::new(), false));
    }

    persona.avatar_url = Some(authorization.url.clone());
    persona.updated_at = now_iso();
    let linked = records
        .iter_mut()
        .enumerate()
        .filter_map(|(index, record)| {
            if record.persona_id.as_deref() == Some(persona_id.as_str()) {
                record.avatar_url = Some(authorization.url.clone());
                record.updated_at = now_iso();
                Some(index)
            } else {
                None
            }
        })
        .collect();
    Ok((persona_id, linked, true))
}

/// Apply an owner-signed avatar command received on a managed agent's observer
/// frame. Persistence happens once under the Desktop store lock;
/// relay reconciliation starts only after the definition and every linked
/// instance are durably in agreement.
#[tauri::command]
pub async fn apply_managed_agent_avatar_request(
    source_pubkey: String,
    authorization_event: String,
    app: AppHandle,
) -> Result<ApplyManagedAgentAvatarResult, String> {
    let owner_keys = app.state::<AppState>().signing_keys()?;
    let owner = owner_keys.public_key();
    let (authorization, issued_at) =
        validate_avatar_authorization(&authorization_event, &owner, &source_pubkey, Utc::now())?;
    let target_pubkey = authorization.target_pubkey.clone();
    let request_id = authorization.request_id.clone();
    let avatar_url = authorization.url.clone();

    let (persona_id, instance_count, targets, applied) = tokio::task::spawn_blocking({
        let app = app.clone();
        move || -> Result<(String, usize, Vec<ReconcileTarget>, bool), String> {
            let state = app.state::<AppState>();
            let _store_guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            let mut personas = load_personas(&app)?;
            let mut records = load_managed_agents(&app)?;
            let (persona_id, linked, applied) =
                apply_avatar_to_state(&mut personas, &mut records, &authorization, issued_at)?;
            if !applied {
                return Ok((persona_id, 0, Vec::new(), false));
            }

            let definitions: Vec<_> = personas
                .iter()
                .cloned()
                .map(AgentDefinition::into_agent_record)
                .collect();
            save_agent_definitions_and_instances(&app, &definitions, &records)?;
            try_regenerate_nest(&app);

            let targets = linked
                .iter()
                .map(|index| {
                    let record = &records[*index];
                    ReconcileTarget {
                        pubkey: record.pubkey.clone(),
                        data: ProfileReconcileData {
                            private_key_nsec: record.private_key_nsec.clone(),
                            name: record.name.clone(),
                            relay_url: record.relay_url.clone(),
                            avatar_url: record.avatar_url.clone(),
                            auth_tag: record.auth_tag.clone(),
                            pubkey: record.pubkey.clone(),
                            agent_command: record.agent_command.clone(),
                            persona_id: record.persona_id.clone(),
                        },
                    }
                })
                .collect();
            Ok((persona_id, linked.len(), targets, true))
        }
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))??;

    let mut reconciled_count = 0;
    if applied {
        let state = app.state::<AppState>();
        for target in targets {
            match reconcile_agent_profile(&state, &app, &target.pubkey, &target.data).await {
                Ok(()) => reconciled_count += 1,
                Err(error) => eprintln!(
                    "buzz-desktop: avatar reconcile failed for agent {}: {error}",
                    target.pubkey
                ),
            }
        }
    }

    let receipt = AvatarApplyReceipt {
        receipt_type: "agent_avatar_apply_receipt",
        request_id: &request_id,
        target_pubkey: &target_pubkey,
        url: &avatar_url,
        applied,
        persona_id: &persona_id,
        instance_count,
        reconciled_count,
    };
    let receipt_content = serde_json::to_string(&receipt)
        .map_err(|error| format!("serialize avatar apply receipt: {error}"))?;
    let receipt_d_tag = format!("{AVATAR_RECEIPT_D_TAG_PREFIX}{request_id}");
    let receipt_tag = Tag::parse(["d", receipt_d_tag.as_str()])
        .map_err(|error| format!("build avatar apply receipt tag: {error}"))?;
    let receipt_builder =
        EventBuilder::new(Kind::Custom(AVATAR_AUTHORIZATION_KIND), receipt_content)
            .tags([receipt_tag]);
    if let Err(error) = crate::relay::submit_event_with_keys(
        receipt_builder,
        &app.state::<AppState>(),
        &owner_keys,
        None,
    )
    .await
    {
        eprintln!("buzz-desktop: failed to publish avatar apply receipt: {error}");
    }

    Ok(ApplyManagedAgentAvatarResult {
        applied,
        target_pubkey,
        persona_id,
        instance_count,
        reconciled_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Tag, ToBech32};
    use std::collections::BTreeMap;

    fn signed_authorization(
        owner: &Keys,
        target: &PublicKey,
        issued_at: DateTime<Utc>,
        url: &str,
    ) -> String {
        let authorization = serde_json::json!({
            "type": AVATAR_APPLY_REQUEST_KIND,
            "requestId": uuid::Uuid::new_v4().to_string(),
            "targetPubkey": target.to_hex(),
            "url": url,
            "issuedAt": issued_at.to_rfc3339(),
        });
        EventBuilder::new(
            Kind::Custom(AVATAR_AUTHORIZATION_KIND),
            authorization.to_string(),
        )
        .custom_created_at(nostr::Timestamp::from(issued_at.timestamp() as u64))
        .tags([Tag::parse(["d", AVATAR_AUTHORIZATION_D_TAG]).unwrap()])
        .sign_with_keys(owner)
        .unwrap()
        .as_json()
    }

    fn persona(id: &str, updated_at: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.into(),
            display_name: id.into(),
            avatar_url: Some("https://relay.example/old.png".into()),
            system_prompt: String::new(),
            runtime: None,
            model: None,
            provider: None,
            claude_account_id: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            env_vars: BTreeMap::new(),
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            parallelism: None,
            created_at: updated_at.into(),
            updated_at: updated_at.into(),
        }
    }

    fn instance(definition: &AgentDefinition, keys: &Keys) -> ManagedAgentRecord {
        let persona_id = definition.id.clone();
        let mut record = definition.clone().into_agent_record();
        record.pubkey = keys.public_key().to_hex();
        record.private_key_nsec = keys.secret_key().to_bech32().unwrap();
        record.persona_id = Some(persona_id);
        record
    }

    #[test]
    fn authorization_requires_current_owner_valid_source_and_https() {
        let now = Utc::now();
        let owner = Keys::generate();
        let other_owner = Keys::generate();
        let target = Keys::generate();
        let other_target = Keys::generate();
        let event = signed_authorization(
            &owner,
            &target.public_key(),
            now,
            "https://relay.example/portrait.png",
        );
        assert!(validate_avatar_authorization(
            &event,
            &owner.public_key(),
            &target.public_key().to_hex(),
            now,
        )
        .is_ok());
        assert!(validate_avatar_authorization(
            &event,
            &other_owner.public_key(),
            &target.public_key().to_hex(),
            now,
        )
        .is_err());
        assert!(validate_avatar_authorization(
            &event,
            &owner.public_key(),
            &other_target.public_key().to_hex(),
            now,
        )
        .is_ok());

        let insecure = signed_authorization(
            &owner,
            &target.public_key(),
            now,
            "http://relay.example/portrait.png",
        );
        assert!(validate_avatar_authorization(
            &insecure,
            &owner.public_key(),
            &target.public_key().to_hex(),
            now,
        )
        .is_err());
    }

    #[test]
    fn avatar_apply_updates_definition_and_every_linked_instance_before_reconcile() {
        let issued_at = Utc::now();
        let old = (issued_at - Duration::minutes(1)).to_rfc3339();
        let target = Keys::generate();
        let sibling = Keys::generate();
        let unrelated = Keys::generate();
        let mut personas = vec![persona("bolt", &old), persona("other", &old)];
        let mut records = vec![
            instance(&personas[0], &target),
            instance(&personas[0], &sibling),
            instance(&personas[1], &unrelated),
        ];
        let authorization = AvatarAuthorization {
            request_type: AVATAR_APPLY_REQUEST_KIND.into(),
            request_id: uuid::Uuid::new_v4().to_string(),
            target_pubkey: target.public_key().to_hex(),
            url: "https://relay.example/new.png".into(),
            issued_at: issued_at.to_rfc3339(),
        };

        let (persona_id, linked, applied) =
            apply_avatar_to_state(&mut personas, &mut records, &authorization, issued_at).unwrap();

        assert!(applied);
        assert_eq!(persona_id, "bolt");
        assert_eq!(linked, vec![0, 1]);
        assert_eq!(
            personas[0].avatar_url.as_deref(),
            Some("https://relay.example/new.png")
        );
        assert_eq!(records[0].avatar_url, personas[0].avatar_url);
        assert_eq!(records[1].avatar_url, personas[0].avatar_url);
        assert_eq!(
            records[2].avatar_url.as_deref(),
            Some("https://relay.example/old.png")
        );

        let (_, replay_links, replay_applied) =
            apply_avatar_to_state(&mut personas, &mut records, &authorization, issued_at).unwrap();
        assert!(!replay_applied);
        assert!(replay_links.is_empty());
    }
}
