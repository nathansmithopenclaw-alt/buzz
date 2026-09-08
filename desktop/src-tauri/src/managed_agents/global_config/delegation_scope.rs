//! Explicit target and field limits. Version 4 prevents older clients from
//! interpreting a bounded grant as an unbounded v1 permission.
use serde::{Deserialize, Serialize};

use super::{
    canonical_relay_scope, AgentManagementDelegationPayload, AgentManagementDelegationPayloadV1,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Exact current targets and independently authorized existing-agent fields.
pub struct AgentEditScope {
    pub target_pubkeys: Vec<String>,
    pub allowed_fields: Vec<String>,
    pub allow_self_edit: bool,
}

impl AgentEditScope {
    /// Validate supported fields and canonicalize signed scope contents.
    pub fn canonicalize(&mut self) -> Result<(), String> {
        for target in &mut self.target_pubkeys {
            *target = nostr::PublicKey::parse(target.trim())
                .map_err(|_| "invalid delegation target pubkey")?
                .to_hex();
        }
        self.target_pubkeys.sort();
        self.target_pubkeys.dedup();
        self.allowed_fields.sort();
        self.allowed_fields.dedup();
        if self.target_pubkeys.is_empty() || self.allowed_fields.is_empty() {
            return Err("Select at least one target agent and allowed field".into());
        }
        if self
            .allowed_fields
            .iter()
            .any(|field| !matches!(field.as_str(), "displayName" | "systemPrompt"))
        {
            return Err("Unsupported delegated edit field".into());
        }
        Ok(())
    }

    /// Every affected target and requested field must be explicitly permitted.
    pub fn allows(&self, source: &str, targets: &[String], fields: &[String]) -> bool {
        !targets.is_empty()
            && !fields.is_empty()
            && targets.iter().all(|target| {
                self.target_pubkeys.contains(target) && (self.allow_self_edit || target != source)
            })
            && fields
                .iter()
                .all(|field| self.allowed_fields.contains(field))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
/// Owner-signed singular v4 grant bound to a community and edit scope.
pub struct ScopedDelegationPayload {
    pub version: u32,
    #[serde(flatten)]
    pub grant: AgentManagementDelegationPayloadV1,
    pub relay_scope: String,
    pub agent_edit: AgentEditScope,
}

impl ScopedDelegationPayload {
    /// Validate supported fields and canonicalize signed scope contents.
    pub fn canonicalize(&mut self) -> Result<(), String> {
        if self.version != 4 {
            return Err("unknown scoped delegation version".into());
        }
        let mut grant = AgentManagementDelegationPayload::V1(self.grant.clone());
        grant.canonicalize()?;
        let AgentManagementDelegationPayload::V1(grant) = grant else {
            return Err("Expected a singular delegation grant".into());
        };
        self.grant = grant;
        if self.relay_scope.trim().is_empty() {
            return Err("delegation relay scope is required".into());
        }
        self.relay_scope = canonical_relay_scope(&self.relay_scope);
        self.agent_edit.canonicalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_field_and_self_edit_limits_are_conjunctive() {
        let scope = AgentEditScope {
            target_pubkeys: vec!["worker".into(), "manager".into()],
            allowed_fields: vec!["displayName".into()],
            allow_self_edit: false,
        };
        for (targets, fields, expected) in [
            (vec!["worker"], vec!["displayName"], true),
            (vec!["outsider"], vec!["displayName"], false),
            (vec!["worker", "outsider"], vec!["displayName"], false),
            (vec!["manager"], vec!["displayName"], false),
            (vec!["worker"], vec!["systemPrompt"], false),
            (vec!["worker"], vec!["displayName", "runtime"], false),
            (vec![], vec!["displayName"], false),
            (vec!["worker"], vec![], false),
        ] {
            assert_eq!(
                scope.allows(
                    "manager",
                    &targets.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                    &fields.iter().map(|s| s.to_string()).collect::<Vec<_>>()
                ),
                expected
            );
        }
        let mut self_allowed = scope;
        self_allowed.allow_self_edit = true;
        assert!(self_allowed.allows("manager", &["manager".into()], &["displayName".into()]));
    }
}
