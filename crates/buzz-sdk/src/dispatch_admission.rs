//! Immutable relay admission evidence. Current membership still controls all new
//! writes and access. A baseline records the role observed during an explicit
//! legacy migration; it never claims to reconstruct an original historical role.
use crate::tracker::{TrackerAuthorization, TrackerRole};
use buzz_core::kind::{
    KIND_DISPATCH_ADMISSION, KIND_DISPATCH_RECORD, KIND_TRACKER_COMMENT, KIND_TRACKER_ISSUE,
    KIND_TRACKER_REVISION,
};
use nostr::{Event, EventBuilder, EventId, Kind, PublicKey, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Evidence boundary for the role, independent of author-controlled created_at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionBasis {
    /// Captured under the membership lock in the original event transaction.
    Admission,
    /// Current role observed by an explicitly requested legacy baseline.
    LegacyBaseline,
}
/// Immutable per-event authority attested by this workspace's relay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchAdmission {
    /// Version one.
    pub v: u8,
    /// Admitted workspace.
    pub workspace: Uuid,
    /// Exact signed event.
    pub event_id: String,
    /// Exact author of that event.
    pub author: String,
    /// Member, maintainer, or none (an explicitly denied legacy role).
    pub role: String,
    /// Relay clock at observation, never the author's event clock.
    pub observed_at: u64,
    /// Original admission or an explicit legacy baseline.
    pub basis: AdmissionBasis,
}
/// Whether this event participates in Dispatch historical authorization.
pub fn requires_dispatch_admission(kind: u32) -> bool {
    matches!(
        kind,
        KIND_TRACKER_ISSUE | KIND_TRACKER_REVISION | KIND_TRACKER_COMMENT | KIND_DISPATCH_RECORD
    )
}
/// Valid task messages also retain their author's admission, without admitting
/// ordinary conversation messages or malformed task-looking text.
pub fn requires_dispatch_event_admission(event: &Event) -> bool {
    requires_dispatch_admission(event.kind.as_u16() as u32)
        || (event.kind.as_u16() == 9
            && crate::task_envelope::TaskEnvelopeEvent::from_event(event).is_ok())
}

/// Build an immutable receipt; only the relay may publish kind 40027.
pub fn build_dispatch_admission(value: &DispatchAdmission) -> Result<EventBuilder, String> {
    if value.v != 1
        || value.workspace.is_nil()
        || !matches!(value.role.as_str(), "member" | "maintainer" | "none")
        || value.observed_at == 0
    {
        return Err("invalid Dispatch admission".into());
    }
    EventId::from_hex(&value.event_id).map_err(|e| e.to_string())?;
    PublicKey::from_hex(&value.author).map_err(|e| e.to_string())?;
    if value.role == "none" && value.basis == AdmissionBasis::Admission {
        return Err("new admission requires a workspace role".into());
    }
    let tags = [
        ["h".into(), value.workspace.to_string()],
        ["e".into(), value.event_id.clone()],
        ["p".into(), value.author.clone()],
    ];
    Ok(EventBuilder::new(
        Kind::Custom(KIND_DISPATCH_ADMISSION as u16),
        serde_json::to_string(value).map_err(|e| e.to_string())?,
    )
    .tags(
        tags.into_iter()
            .map(|tag| Tag::parse(tag).expect("two-element tags")),
    )
    .custom_created_at(Timestamp::from(value.observed_at)))
}
/// Coverage visible to clients; unsealed legacy records must never be presented
/// as historically authorized or enable unattended automation.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionCoverage {
    /// Exact event IDs with valid, unambiguous relay evidence.
    pub sealed: BTreeSet<String>,
    /// Relay observation times for sealed events, after all receipt checks.
    #[serde(skip)]
    pub observed_at: BTreeMap<String, u64>,
    /// Signed legacy baselines among sealed events.
    pub legacy_baselines: usize,
    /// Dispatch events without historical evidence.
    pub unsealed: usize,
    /// Invalid or contradictory receipt descriptions.
    pub diagnostics: Vec<String>,
}
/// Apply signed receipts from the expected relay only. Conflicting receipts fail
/// closed for their exact event; a receipt never grants its author a current role.
pub fn apply_dispatch_admissions(
    events: &[Event],
    workspace: Uuid,
    relay: PublicKey,
    authorization: &mut TrackerAuthorization,
    now: u64,
) -> AdmissionCoverage {
    let mut result = AdmissionCoverage::default();
    let sources: BTreeMap<_, _> = events
        .iter()
        .filter(|event| {
            requires_dispatch_event_admission(event)
                && event
                    .tags
                    .iter()
                    .any(|tag| tag.as_slice() == ["h", &workspace.to_string()])
        })
        .map(|event| (event.id.to_hex(), event))
        .collect();
    let mut receipts = BTreeMap::<String, Vec<DispatchAdmission>>::new();
    for event in events.iter().filter(|event| {
        event.kind.as_u16() as u32 == KIND_DISPATCH_ADMISSION
            && event
                .tags
                .iter()
                .any(|tag| tag.as_slice() == ["h", &workspace.to_string()])
    }) {
        let checked: Result<DispatchAdmission, String> = (|| {
            if event.pubkey != relay {
                return Err("unexpected admission signer".into());
            }
            event.verify().map_err(|e| e.to_string())?;
            let value: DispatchAdmission =
                serde_json::from_str(&event.content).map_err(|e| e.to_string())?;
            if value.workspace != workspace
                || value.observed_at > now
                || event.created_at.as_secs() != value.observed_at
            {
                return Err("admission scope or observation time mismatch".into());
            }
            let expected = build_dispatch_admission(&value)?.build(event.pubkey);
            if expected.tags != event.tags {
                return Err("admission tags mismatch".into());
            }
            if let Some(source) = sources.get(&value.event_id) {
                if source.pubkey.to_hex() != value.author
                    || !source
                        .tags
                        .iter()
                        .any(|tag| tag.as_slice() == ["h", &workspace.to_string()])
                {
                    return Err("admission source mismatch".into());
                }
            }
            Ok(value)
        })();
        match checked {
            Ok(value) => receipts
                .entry(value.event_id.clone())
                .or_default()
                .push(value),
            Err(error) => result.diagnostics.push(format!("{}: {error}", event.id)),
        }
    }
    for (id, values) in receipts {
        let first = &values[0];
        if values.iter().any(|value| value != first) {
            authorization.admit_event(id.clone(), first.author.clone(), None);
            result
                .diagnostics
                .push(format!("{id}: conflicting admission receipts"));
            continue;
        }
        let role = match first.role.as_str() {
            "maintainer" => Some(TrackerRole::Maintainer),
            "member" => Some(TrackerRole::Member),
            _ => None,
        };
        authorization.admit_event(id.clone(), first.author.clone(), role);
        if sources.contains_key(&id) {
            result.observed_at.insert(id.clone(), first.observed_at);
            result.sealed.insert(id);
            result.legacy_baselines += usize::from(first.basis == AdmissionBasis::LegacyBaseline);
        }
    }
    result.unsealed = sources.len().saturating_sub(result.sealed.len());
    result
}

/// Historical task admission with current-roster compatibility for explicitly
/// unsealed legacy messages. The caller must separately check the current
/// viewer's access and trust the roster's relay issuer.
pub fn admitted_task_events(
    events: &[Event],
    workspace: Uuid,
    roster: &Event,
    now: u64,
) -> (
    Vec<crate::task_envelope::TaskEnvelopeEvent>,
    AdmissionCoverage,
) {
    let mut roles = Vec::new();
    for tag in roster.tags.iter() {
        let values = tag.as_slice();
        if values.first().map(String::as_str) == Some("p") {
            if let Some(author) = values.get(1) {
                roles.push((author.clone(), TrackerRole::Member));
            }
        }
    }
    let mut authorization = TrackerAuthorization::new(roles);
    let coverage =
        apply_dispatch_admissions(events, workspace, roster.pubkey, &mut authorization, now);
    let tasks = events
        .iter()
        .filter(|event| {
            authorization
                .event_role(&event.id.to_hex(), &event.pubkey.to_hex())
                .is_some()
        })
        .filter_map(|event| crate::task_envelope::TaskEnvelopeEvent::from_event(event).ok())
        .filter(|event| event.channel_id == workspace)
        .collect();
    (tasks, coverage)
}

#[cfg(test)]
mod tests;
