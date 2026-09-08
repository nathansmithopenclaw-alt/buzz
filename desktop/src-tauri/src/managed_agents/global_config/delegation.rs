//! Owner-signed standing-delegation grant payload (NIP-SD).
//!
//! v1 is a single grantee plus boolean capability bits. v2 is per-capability
//! subjects (`none` / `set` / `all`). Unknown versions fail loud.

use nostr::{Event, JsonUtil, Kind, PublicKey};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Independently revocable capability bits for a v1 grant.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentManagementDelegationCapabilities {
    #[serde(default)]
    pub agent_config: bool,
    #[serde(default)]
    pub global_config: bool,
    #[serde(default)]
    pub teams: bool,
    #[serde(default)]
    pub agent_creation: bool,
    #[serde(default)]
    pub env_vars: bool,
}

impl AgentManagementDelegationCapabilities {
    pub fn is_empty(&self) -> bool {
        !self.agent_config
            && !self.global_config
            && !self.teams
            && !self.agent_creation
            && !self.env_vars
    }
}

/// Who a v2 capability applies to.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum CapabilitySubject {
    #[default]
    None,
    All,
    Set {
        grantees: Vec<String>,
    },
}

impl CapabilitySubject {
    pub fn allows(&self, source_pubkey: &str) -> bool {
        match self {
            Self::None => false,
            Self::All => true,
            Self::Set { grantees } => grantees.iter().any(|grantee| grantee == source_pubkey),
        }
    }

    fn canonicalize(&mut self) -> Result<(), String> {
        match self {
            Self::None | Self::All => Ok(()),
            Self::Set { grantees } => {
                let mut canonical = Vec::with_capacity(grantees.len());
                for grantee in grantees.iter() {
                    let parsed = PublicKey::parse(grantee.trim())
                        .map_err(|error| format!("invalid delegation grantee pubkey: {error}"))?;
                    let hex = parsed.to_hex();
                    if !canonical.iter().any(|existing| existing == &hex) {
                        canonical.push(hex);
                    }
                }
                canonical.sort();
                if canonical.is_empty() {
                    *self = Self::None;
                } else {
                    *grantees = canonical;
                }
                Ok(())
            }
        }
    }
}

/// Per-capability subjects for a v2 grant.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentManagementDelegationSubjects {
    #[serde(default)]
    pub agent_config: CapabilitySubject,
    #[serde(default)]
    pub global_config: CapabilitySubject,
    #[serde(default)]
    pub teams: CapabilitySubject,
    #[serde(default)]
    pub agent_creation: CapabilitySubject,
    #[serde(default)]
    pub env_vars: CapabilitySubject,
}

impl AgentManagementDelegationSubjects {
    #[cfg(test)]
    pub fn all_agents() -> Self {
        Self {
            agent_config: CapabilitySubject::All,
            global_config: CapabilitySubject::All,
            teams: CapabilitySubject::All,
            agent_creation: CapabilitySubject::All,
            env_vars: CapabilitySubject::All,
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self.agent_config, CapabilitySubject::None)
            && matches!(self.global_config, CapabilitySubject::None)
            && matches!(self.teams, CapabilitySubject::None)
            && matches!(self.agent_creation, CapabilitySubject::None)
            && matches!(self.env_vars, CapabilitySubject::None)
    }

    pub fn canonicalize(&mut self) -> Result<(), String> {
        self.agent_config.canonicalize()?;
        self.global_config.canonicalize()?;
        self.teams.canonicalize()?;
        self.agent_creation.canonicalize()?;
        self.env_vars.canonicalize()
    }
}

/// Canonical v1 content signed by the Desktop owner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentManagementDelegationPayloadV1 {
    pub grantee_pubkey: String,
    pub audit_channel_id: String,
    pub capabilities: AgentManagementDelegationCapabilities,
}

/// Canonical v2 content signed by the Desktop owner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentManagementDelegationPayloadV2 {
    pub version: u32,
    pub audit_channel_id: String,
    pub capabilities: AgentManagementDelegationSubjects,
}

/// Canonical content of the owner-signed `kind:30078` grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentManagementDelegationPayload {
    V1(AgentManagementDelegationPayloadV1),
    V2(AgentManagementDelegationPayloadV2),
    V4(super::delegation_scope::ScopedDelegationPayload),
}

impl AgentManagementDelegationPayload {
    #[cfg(test)]
    pub fn v2_all_agents(audit_channel_id: String) -> Self {
        Self::V2(AgentManagementDelegationPayloadV2 {
            version: 2,
            audit_channel_id,
            capabilities: AgentManagementDelegationSubjects::all_agents(),
        })
    }

    pub fn audit_channel_id(&self) -> &str {
        match self {
            Self::V1(payload) => payload.audit_channel_id.as_str(),
            Self::V2(payload) => payload.audit_channel_id.as_str(),
            Self::V4(payload) => &payload.grant.audit_channel_id,
        }
    }

    pub fn allows_named(
        &self,
        source_pubkey: &str,
        capability: &str,
        request_has_env_vars: bool,
    ) -> bool {
        match self {
            Self::V4(payload) => {
                payload.grant.grantee_pubkey == source_pubkey
                    && v1_capability_bit(&payload.grant.capabilities, capability)
                    && (!request_has_env_vars || payload.grant.capabilities.env_vars)
            }
            Self::V1(payload) => {
                payload.grantee_pubkey == source_pubkey
                    && v1_capability_bit(&payload.capabilities, capability)
                    && (!request_has_env_vars || payload.capabilities.env_vars)
            }
            Self::V2(payload) => {
                v2_capability_subject(&payload.capabilities, capability).allows(source_pubkey)
                    && (!request_has_env_vars
                        || payload.capabilities.env_vars.allows(source_pubkey))
            }
        }
    }

    pub fn canonicalize(&mut self) -> Result<(), String> {
        match self {
            Self::V4(payload) => payload.canonicalize(),
            Self::V1(payload) => {
                let grantee = PublicKey::parse(payload.grantee_pubkey.trim())
                    .map_err(|error| format!("invalid delegation grantee pubkey: {error}"))?;
                payload.grantee_pubkey = grantee.to_hex();
                payload.audit_channel_id = uuid::Uuid::parse_str(payload.audit_channel_id.trim())
                    .map_err(|_| "invalid delegation audit channel UUID".to_string())?
                    .to_string();
                Ok(())
            }
            Self::V2(payload) => {
                if payload.version != 2 {
                    return Err(format!(
                        "unknown delegation grant version: {}",
                        payload.version
                    ));
                }
                payload.audit_channel_id = uuid::Uuid::parse_str(payload.audit_channel_id.trim())
                    .map_err(|_| "invalid delegation audit channel UUID".to_string())?
                    .to_string();
                payload.capabilities.canonicalize()
            }
        }
    }
}

fn v1_capability_bit(
    capabilities: &AgentManagementDelegationCapabilities,
    capability: &str,
) -> bool {
    match capability {
        "agent_config" => capabilities.agent_config,
        "agent_creation" => capabilities.agent_creation,
        "env_vars" => capabilities.env_vars,
        "global_config" => capabilities.global_config,
        "teams" => capabilities.teams,
        _ => false,
    }
}

fn v2_capability_subject<'a>(
    capabilities: &'a AgentManagementDelegationSubjects,
    capability: &str,
) -> &'a CapabilitySubject {
    match capability {
        "agent_config" => &capabilities.agent_config,
        "agent_creation" => &capabilities.agent_creation,
        "env_vars" => &capabilities.env_vars,
        "global_config" => &capabilities.global_config,
        "teams" => &capabilities.teams,
        _ => {
            static NONE: CapabilitySubject = CapabilitySubject::None;
            &NONE
        }
    }
}

impl Serialize for AgentManagementDelegationPayload {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::V1(payload) => payload.serialize(serializer),
            Self::V2(payload) => payload.serialize(serializer),
            Self::V4(payload) => payload.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for AgentManagementDelegationPayload {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value.get("version") {
            None => serde_json::from_value(value)
                .map(Self::V1)
                .map_err(serde::de::Error::custom),
            Some(serde_json::Value::Number(number)) if number.as_u64() == Some(2) => {
                serde_json::from_value(value)
                    .map(Self::V2)
                    .map_err(serde::de::Error::custom)
            }
            Some(serde_json::Value::Number(number)) if number.as_u64() == Some(4) => {
                serde_json::from_value(value)
                    .map(Self::V4)
                    .map_err(serde::de::Error::custom)
            }
            Some(other) => Err(serde::de::Error::custom(format!(
                "unknown delegation grant version: {other}"
            ))),
        }
    }
}

/// Desktop-owned standing delegation. The opaque event must be signed by the
/// current owner and its content must exactly match `payload`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentManagementDelegationGrant {
    pub payload: AgentManagementDelegationPayload,
    pub signed_event: Option<String>,
}

impl Serialize for AgentManagementDelegationGrant {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut value = serde_json::to_value(&self.payload).map_err(serde::ser::Error::custom)?;
        if let Some(event) = &self.signed_event {
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "signedEvent".to_string(),
                    serde_json::Value::String(event.clone()),
                );
            }
        }
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AgentManagementDelegationGrant {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut value = serde_json::Value::deserialize(deserializer)?;
        let signed_event =
            value
                .as_object_mut()
                .and_then(|object| match object.remove("signedEvent") {
                    Some(serde_json::Value::String(event)) => Some(event),
                    Some(serde_json::Value::Null) | None => None,
                    Some(_) => None,
                });
        let payload = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Self {
            payload,
            signed_event,
        })
    }
}

impl AgentManagementDelegationGrant {
    pub fn singular_grantee_pubkey(&self) -> Option<&str> {
        match &self.payload {
            AgentManagementDelegationPayload::V1(payload) => Some(payload.grantee_pubkey.as_str()),
            AgentManagementDelegationPayload::V2(_) => None,
            AgentManagementDelegationPayload::V4(payload) => Some(&payload.grant.grantee_pubkey),
        }
    }

    pub fn validate_for_grantee_key(
        &self,
        grantee_pubkey: &str,
        owner: &PublicKey,
    ) -> Result<(), String> {
        let key = PublicKey::parse(grantee_pubkey.trim())
            .map_err(|error| format!("invalid delegation map grantee pubkey: {error}"))?
            .to_hex();
        if key != grantee_pubkey {
            return Err("delegation map grantee key is not canonical lowercase hex".into());
        }
        if self.singular_grantee_pubkey() != Some(grantee_pubkey) {
            return Err("delegation map key does not match its singular signed grant".into());
        }
        self.validate(owner)
    }

    pub fn validate(&self, owner: &PublicKey) -> Result<(), String> {
        match &self.payload {
            AgentManagementDelegationPayload::V4(payload) => {
                let mut canonical = payload.clone();
                canonical.canonicalize()?;
                if &canonical != payload {
                    return Err("delegation scope is not canonical".into());
                }
            }
            AgentManagementDelegationPayload::V1(payload) => {
                let grantee = PublicKey::parse(payload.grantee_pubkey.trim())
                    .map_err(|error| format!("invalid delegation grantee pubkey: {error}"))?;
                if grantee.to_hex() != payload.grantee_pubkey {
                    return Err("delegation grantee pubkey is not canonical lowercase hex".into());
                }
            }
            AgentManagementDelegationPayload::V2(payload) => {
                if payload.version != 2 {
                    return Err(format!(
                        "unknown delegation grant version: {}",
                        payload.version
                    ));
                }
                let mut canonical = payload.capabilities.clone();
                canonical.canonicalize()?;
                if canonical != payload.capabilities {
                    return Err("delegation capability subjects are not canonical".into());
                }
            }
        }
        uuid::Uuid::parse_str(self.payload.audit_channel_id().trim()).map_err(|_| {
            format!(
                "invalid delegation audit channel UUID: {}",
                self.payload.audit_channel_id()
            )
        })?;
        let raw = self
            .signed_event
            .as_deref()
            .ok_or("delegation grant is not owner-signed")?;
        let event = Event::from_json(raw)
            .map_err(|error| format!("invalid delegation signature event: {error}"))?;
        if event.kind != Kind::Custom(30_078) || event.pubkey != *owner || event.verify().is_err() {
            return Err("delegation grant signature is invalid for this owner".into());
        }
        let signed_payload: AgentManagementDelegationPayload = serde_json::from_str(&event.content)
            .map_err(|error| format!("invalid delegation grant content: {error}"))?;
        if signed_payload != self.payload {
            return Err("delegation grant fields do not match the owner signature".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys};

    const AUDIT: &str = "7c07e659-3610-42f4-9a5e-1e9973c09da9";

    fn v1_payload(grantee: String) -> AgentManagementDelegationPayload {
        AgentManagementDelegationPayload::V1(AgentManagementDelegationPayloadV1 {
            grantee_pubkey: grantee,
            audit_channel_id: AUDIT.into(),
            capabilities: AgentManagementDelegationCapabilities {
                agent_creation: true,
                teams: true,
                ..Default::default()
            },
        })
    }

    fn sign(
        owner: &Keys,
        payload: AgentManagementDelegationPayload,
    ) -> AgentManagementDelegationGrant {
        let event = EventBuilder::new(
            Kind::Custom(30_078),
            serde_json::to_string(&payload).unwrap(),
        )
        .sign_with_keys(owner)
        .unwrap();
        AgentManagementDelegationGrant {
            payload,
            signed_event: Some(event.as_json()),
        }
    }

    #[test]
    fn v1_json_round_trips_without_version() {
        let json = serde_json::json!({
            "granteePubkey": "11".repeat(32),
            "auditChannelId": AUDIT,
            "capabilities": { "agent_creation": true, "teams": true }
        });
        let payload: AgentManagementDelegationPayload = serde_json::from_value(json).unwrap();
        match &payload {
            AgentManagementDelegationPayload::V1(v1) => {
                assert!(v1.capabilities.agent_creation);
                assert!(!v1.capabilities.agent_config);
            }
            _ => panic!("expected v1"),
        }
        let encoded = serde_json::to_value(&payload).unwrap();
        assert!(encoded.get("version").is_none());
        assert_eq!(encoded["granteePubkey"], "11".repeat(32));
    }

    #[test]
    fn unknown_version_fails_loud() {
        let json = serde_json::json!({
            "version": 3,
            "auditChannelId": AUDIT,
            "capabilities": {}
        });
        let error = serde_json::from_value::<AgentManagementDelegationPayload>(json)
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown delegation grant version"));
    }

    #[test]
    fn v1_signature_survives_grant_envelope_round_trip() {
        let owner = Keys::generate();
        let grantee = Keys::generate();
        let grant = sign(&owner, v1_payload(grantee.public_key().to_hex()));
        grant.validate(&owner.public_key()).unwrap();
        let bytes = serde_json::to_vec(&grant).unwrap();
        let decoded: AgentManagementDelegationGrant = serde_json::from_slice(&bytes).unwrap();
        decoded.validate(&owner.public_key()).unwrap();
        assert_eq!(decoded, grant);
    }

    #[test]
    fn grant_all_is_one_v2_payload_with_five_all_scopes() {
        let payload = AgentManagementDelegationPayload::v2_all_agents(AUDIT.into());
        let encoded = serde_json::to_value(&payload).unwrap();
        assert_eq!(encoded["version"], 2);
        for name in [
            "agent_config",
            "global_config",
            "teams",
            "agent_creation",
            "env_vars",
        ] {
            assert_eq!(encoded["capabilities"][name]["scope"], "all", "{name}");
        }
        let owner = Keys::generate();
        let grant = sign(&owner, payload);
        grant.validate(&owner.public_key()).unwrap();
        // One signed event, not five — a single kind:30078 JSON object.
        let event = Event::from_json(grant.signed_event.as_ref().unwrap()).unwrap();
        assert_eq!(event.kind, Kind::Custom(30_078));
        let signed: AgentManagementDelegationPayload =
            serde_json::from_str(&event.content).unwrap();
        match signed {
            AgentManagementDelegationPayload::V2(v2) => {
                assert!(matches!(
                    v2.capabilities.agent_config,
                    CapabilitySubject::All
                ));
                assert!(matches!(
                    v2.capabilities.global_config,
                    CapabilitySubject::All
                ));
                assert!(matches!(v2.capabilities.teams, CapabilitySubject::All));
                assert!(matches!(
                    v2.capabilities.agent_creation,
                    CapabilitySubject::All
                ));
                assert!(matches!(v2.capabilities.env_vars, CapabilitySubject::All));
            }
            _ => panic!("expected v2"),
        }
    }

    #[test]
    fn v2_validate_rejects_noncanonical_set_entries() {
        let owner = Keys::generate();
        let grantee = Keys::generate();
        let hex = grantee.public_key().to_hex();
        let payload = AgentManagementDelegationPayload::V2(AgentManagementDelegationPayloadV2 {
            version: 2,
            audit_channel_id: AUDIT.into(),
            capabilities: AgentManagementDelegationSubjects {
                agent_creation: CapabilitySubject::Set {
                    grantees: vec![hex.to_ascii_uppercase(), hex.clone()],
                },
                ..Default::default()
            },
        });
        let grant = sign(&owner, payload);
        let error = grant.validate(&owner.public_key()).unwrap_err();
        assert!(
            error.contains("not canonical"),
            "expected canonicality error, got: {error}"
        );
    }

    #[test]
    fn v2_set_canonicalizes_and_empty_set_becomes_none() {
        let mut payload =
            AgentManagementDelegationPayload::V2(AgentManagementDelegationPayloadV2 {
                version: 2,
                audit_channel_id: AUDIT.into(),
                capabilities: AgentManagementDelegationSubjects {
                    agent_creation: CapabilitySubject::Set {
                        grantees: vec!["BB".repeat(32), "aa".repeat(32), "AA".repeat(32)],
                    },
                    env_vars: CapabilitySubject::Set { grantees: vec![] },
                    ..Default::default()
                },
            });
        payload.canonicalize().unwrap();
        match payload {
            AgentManagementDelegationPayload::V2(v2) => {
                match v2.capabilities.agent_creation {
                    CapabilitySubject::Set { grantees } => {
                        assert_eq!(grantees.len(), 2);
                        assert!(grantees[0] < grantees[1]);
                        assert!(grantees.iter().all(|g| g
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())));
                    }
                    other => panic!("expected set, got {other:?}"),
                }
                assert!(matches!(v2.capabilities.env_vars, CapabilitySubject::None));
            }
            _ => panic!("expected v2"),
        }
    }
}
