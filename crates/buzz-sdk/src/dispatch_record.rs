//! Signed, channel-scoped planning records for the human Dispatch workflow.

use crate::tracker::{ExternalRef, TrackerAuthorization, TrackerRole};
use buzz_core::kind::KIND_DISPATCH_RECORD;
use nostr::{Event, EventBuilder, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// A concrete choice with its consequence visible before a decision is made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionOption {
    /// Stable option key.
    pub id: String,
    /// Human label.
    pub label: String,
    /// Expected consequence, including meaningful tradeoffs.
    pub consequence: String,
}

/// A lightweight outcome checkpoint, independent of issue status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Milestone {
    /// Stable identity within the project plan.
    pub id: Uuid,
    /// Intended result.
    pub title: String,
    /// Optional calendar date (YYYY-MM-DD).
    pub target_date: Option<String>,
    /// Issues contributing to this outcome.
    pub issue_ids: Vec<Uuid>,
}

/// Proof tiers are separate observations; none implies another tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStage {
    /// Executor claim.
    ReportedComplete,
    /// A specific test execution.
    Tested,
    /// Independent review.
    Reviewed,
    /// Code integrated into a named base.
    Merged,
    /// Artifact installed in a named environment.
    Installed,
    /// Observed behavior accepted by the decision maker.
    Accepted,
}

/// Supported shared records. Unknown fields are rejected rather than silently lost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DispatchRecordPayload {
    /// A bounded automation rollout or a pause/preview control for one service.
    Automation {
        /// Explicit scope, routing, expected service signer and limits.
        controls: crate::dispatch_automation::AutomationControls,
    },
    /// A shared view of existing issues; personal views remain local to their owner.
    View {
        /// View name.
        name: String,
        /// Whitelisted Dispatch navigation filters.
        filters: BTreeMap<String, String>,
    },
    /// A question attached to existing work with an explicit decision maker.
    Decision {
        /// Related issue.
        issue_id: Uuid,
        /// Public key authorized to decide.
        decider: String,
        /// Exact question.
        question: String,
        /// Context needed to decide.
        context: String,
        /// Available choices and consequences.
        options: Vec<DecisionOption>,
        /// Source and evidence references; content is never copied implicitly.
        evidence: Vec<ExternalRef>,
        /// Chosen option, absent while awaiting a decision.
        selected_option: Option<String>,
        /// Explanation recorded with the decision.
        rationale: Option<String>,
    },
    /// Project purpose, outcomes, documentation and milestones.
    ProjectPlan {
        /// Existing Tracker project identity.
        project_id: Uuid,
        /// Accountable person.
        owner: Option<String>,
        /// Why this project exists.
        purpose: String,
        /// Intended observable result.
        outcome: String,
        /// Optional calendar date.
        target_date: Option<String>,
        /// Lightweight checkpoints.
        milestones: Vec<Milestone>,
        /// Documentation and conversation references.
        links: Vec<ExternalRef>,
    },
    /// Deliberate activation preserves the original import provenance.
    Activation {
        /// Existing imported issue.
        issue_id: Uuid,
        /// Whether current workflow rules may consider the issue.
        active: bool,
        /// Reason for activating or parking this work.
        reason: String,
    },
    /// A signed observation attached to an artifact and a proof tier.
    Delivery {
        /// Related issue.
        issue_id: Uuid,
        /// Optional execution attempt.
        task_id: Option<Uuid>,
        /// What was observed.
        stage: DeliveryStage,
        /// Exact artifact, commit, report or environment reference.
        artifact: String,
        /// Test or acceptance description, including limits.
        description: String,
        /// Whether this specific observation passed.
        passed: bool,
        /// Time of observation (Unix seconds).
        observed_at: u64,
    },
    /// Human-managed execution capacity, used to explain routing proposals.
    Capacity {
        /// Executor public key.
        executor: String,
        /// Maximum simultaneous unfinished attempts.
        max_active: u32,
        /// Explicit routing capability labels.
        capabilities: Vec<String>,
        /// Manual pause prevents new routing proposals.
        paused: bool,
    },
}

/// One immutable root or complete revision of a planning record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchRecord {
    /// Schema version (currently 1).
    pub v: u8,
    /// Stable record UUID.
    pub record_id: Uuid,
    /// Zero for the root, incremented for each accepted revision.
    pub revision: u32,
    /// Accepted predecessor event, absent only on a root.
    pub parent_event: Option<String>,
    /// Durable action identity for replay detection.
    pub request_id: Uuid,
    /// Explicit archival, preserving history.
    pub archived: bool,
    /// Complete typed payload.
    pub payload: DispatchRecordPayload,
}

/// Signed current record returned by the authoritative fold.
#[derive(Debug, Clone, Serialize)]
pub struct DispatchRecordProjection {
    /// Original creator; remains stable across revisions.
    pub creator: String,
    /// Author of the current revision.
    pub author: String,
    /// Current signed event.
    pub event_id: String,
    /// Original signed event.
    pub root_event: String,
    /// Accepted ancestry, allowing exact retry acknowledgement after a later revision.
    pub accepted_events: Vec<String>,
    /// Time of current revision.
    pub created_at: u64,
    /// Typed record contents.
    pub record: DispatchRecord,
}

/// Canonical records and actionable rejected claims.
#[derive(Debug, Default, Serialize)]
pub struct DispatchRecordFold {
    /// Current record per stable identity, including archived records.
    pub records: BTreeMap<Uuid, DispatchRecordProjection>,
    /// Rejected event IDs and reasons; signed history remains intact.
    pub diagnostics: Vec<String>,
}

fn hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn key(value: &str) -> bool {
    value.len() == 64 && PublicKey::from_hex(value).is_ok()
}
fn bounded(value: &str, max: usize, required: bool) -> bool {
    value.len() <= max && (!required || !value.trim().is_empty())
}
fn links(values: &[ExternalRef]) -> bool {
    values.len() <= 50
        && values
            .iter()
            .all(|link| bounded(&link.kind, 32, true) && bounded(&link.reference, 4096, true))
}
fn date(value: &Option<String>) -> bool {
    value
        .as_ref()
        .is_none_or(|value| chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok())
}
fn slot(payload: &DispatchRecordPayload) -> String {
    match payload {
        DispatchRecordPayload::Automation { controls } => format!(
            "automation:{}:{}",
            controls.service_id, controls.service_pubkey
        ),
        DispatchRecordPayload::View { .. } => "view".into(),
        DispatchRecordPayload::Decision {
            issue_id, decider, ..
        } => format!("decision:{issue_id}:{decider}"),
        DispatchRecordPayload::ProjectPlan { project_id, .. } => format!("project:{project_id}"),
        DispatchRecordPayload::Activation { issue_id, .. } => format!("activation:{issue_id}"),
        DispatchRecordPayload::Delivery {
            issue_id, task_id, ..
        } => format!("delivery:{issue_id}:{task_id:?}"),
        DispatchRecordPayload::Capacity { executor, .. } => format!("capacity:{executor}"),
    }
}
fn controlled(payload: &DispatchRecordPayload) -> bool {
    matches!(
        payload,
        DispatchRecordPayload::Automation { .. }
            | DispatchRecordPayload::Activation { .. }
            | DispatchRecordPayload::Capacity { .. }
            | DispatchRecordPayload::Delivery {
                stage: DeliveryStage::Reviewed | DeliveryStage::Accepted,
                ..
            }
    )
}

/// Validate a record before signing and again when reading untrusted history.
pub fn validate_dispatch_record(record: &DispatchRecord) -> Result<(), String> {
    if record.v != 1
        || record.record_id.is_nil()
        || record.request_id.is_nil()
        || (record.revision == 0) != record.parent_event.is_none()
        || record.parent_event.as_ref().is_some_and(|id| !hash(id))
    {
        return Err("invalid planning record identity, version or predecessor".into());
    }
    let valid = match &record.payload {
        DispatchRecordPayload::Automation { controls } => controls.validate(),
        DispatchRecordPayload::View { name, filters } => {
            bounded(name, 80, true)
                && filters.len() <= 16
                && filters.iter().all(|(k, v)| {
                    [
                        "view",
                        "status",
                        "priority",
                        "assignee",
                        "project",
                        "label",
                        "query",
                        "group",
                        "sort",
                        "direction",
                        "attention",
                    ]
                    .contains(&k.as_str())
                        && bounded(v, 256, true)
                })
        }
        DispatchRecordPayload::Decision {
            issue_id,
            decider,
            question,
            context,
            options,
            evidence,
            selected_option,
            rationale,
        } => {
            !issue_id.is_nil()
                && key(decider)
                && bounded(question, 256, true)
                && bounded(context, 16000, false)
                && (2..=8).contains(&options.len())
                && options.iter().all(|option| {
                    bounded(&option.id, 40, true)
                        && bounded(&option.label, 120, true)
                        && bounded(&option.consequence, 2000, false)
                })
                && options
                    .iter()
                    .map(|o| &o.id)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == options.len()
                && links(evidence)
                && selected_option
                    .as_ref()
                    .is_none_or(|id| options.iter().any(|option| &option.id == id))
                && rationale
                    .as_ref()
                    .is_none_or(|value| bounded(value, 4000, true))
                && (selected_option.is_none() || rationale.is_some())
        }
        DispatchRecordPayload::ProjectPlan {
            project_id,
            owner,
            purpose,
            outcome,
            target_date,
            milestones,
            links: refs,
        } => {
            !project_id.is_nil()
                && owner.as_ref().is_none_or(|value| key(value))
                && bounded(purpose, 16000, false)
                && bounded(outcome, 4000, true)
                && date(target_date)
                && milestones.len() <= 50
                && milestones
                    .iter()
                    .map(|milestone| milestone.id)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == milestones.len()
                && milestones.iter().all(|milestone| {
                    !milestone.id.is_nil()
                        && bounded(&milestone.title, 256, true)
                        && date(&milestone.target_date)
                        && milestone.issue_ids.len() <= 500
                        && milestone.issue_ids.iter().all(|id| !id.is_nil())
                        && milestone
                            .issue_ids
                            .iter()
                            .collect::<std::collections::BTreeSet<_>>()
                            .len()
                            == milestone.issue_ids.len()
                })
                && links(refs)
        }
        DispatchRecordPayload::Activation {
            issue_id, reason, ..
        } => !issue_id.is_nil() && bounded(reason, 2000, true),
        DispatchRecordPayload::Delivery {
            issue_id,
            artifact,
            description,
            observed_at,
            task_id,
            ..
        } => {
            !issue_id.is_nil()
                && bounded(artifact, 4096, true)
                && bounded(description, 16000, true)
                && *observed_at > 0
                && task_id.is_none_or(|id| !id.is_nil())
        }
        DispatchRecordPayload::Capacity {
            executor,
            max_active,
            capabilities,
            ..
        } => {
            key(executor)
                && *max_active <= 32
                && capabilities.len() <= 50
                && capabilities.iter().all(|value| bounded(value, 80, true))
        }
    };
    if valid {
        Ok(())
    } else {
        Err("planning record contains missing, invalid or oversized fields".into())
    }
}

/// Build a channel-scoped immutable event; signing remains with the caller's identity.
pub fn build_dispatch_record(
    workspace: Uuid,
    record: &DispatchRecord,
) -> Result<EventBuilder, String> {
    validate_dispatch_record(record)?;
    if workspace.is_nil() {
        return Err("workspace is required".into());
    }
    let mut tags = vec![
        vec!["h".into(), workspace.to_string()],
        vec!["record".into(), record.record_id.to_string()],
        vec!["request".into(), record.request_id.to_string()],
    ];
    if let Some(parent) = &record.parent_event {
        tags.push(vec![
            "e".into(),
            parent.clone(),
            String::new(),
            "parent".into(),
        ]);
    }
    let tags = tags
        .into_iter()
        .map(Tag::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(EventBuilder::new(
        Kind::Custom(KIND_DISPATCH_RECORD as u16),
        serde_json::to_string(record).map_err(|e| e.to_string())?,
    )
    .tags(tags))
}

fn parse(event: &Event, workspace: Uuid, now: u64) -> Result<DispatchRecord, String> {
    if event.kind.as_u16() as u32 != KIND_DISPATCH_RECORD
        || event.created_at.as_secs() > now
        || event.content.len() > 65536
    {
        return Err("invalid planning event kind, time or size".into());
    }
    event.verify().map_err(|e| e.to_string())?;
    let record: DispatchRecord = serde_json::from_str(&event.content).map_err(|e| e.to_string())?;
    if matches!(&record.payload,DispatchRecordPayload::Delivery{observed_at,..} if *observed_at>now)
    {
        return Err("delivery observation is in the future".into());
    }
    if matches!(&record.payload,DispatchRecordPayload::Automation{controls} if controls.expires_at.is_some_and(|deadline|deadline>event.created_at.as_secs().saturating_add(86400)))
    {
        return Err("automation approvals expire within 24 hours of their signed revision".into());
    }
    let rebuilt = build_dispatch_record(workspace, &record)?.build(event.pubkey);
    let semantic_tags = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) != Some("auth"))
        .collect::<Vec<_>>();
    let auth_count = event.tags.len() - semantic_tags.len();
    if auth_count > 1 || rebuilt.tags.iter().collect::<Vec<_>>() != semantic_tags {
        return Err("planning record tags do not match its signed scope".into());
    }
    Ok(record)
}

/// Fold signed records with immutable identity, predecessor and decision authority.
/// Authorization is the caller's authoritative workspace membership configuration.
pub fn fold_dispatch_records(
    events: impl IntoIterator<Item = Event>,
    workspace: Uuid,
    authorization: &TrackerAuthorization,
    now: u64,
) -> DispatchRecordFold {
    let mut result = DispatchRecordFold::default();
    let mut groups = BTreeMap::<Uuid, Vec<(Event, DispatchRecord)>>::new();
    for event in events {
        if event.kind.as_u16() as u32 != KIND_DISPATCH_RECORD {
            continue;
        }
        match parse(&event, workspace, now) {
            Ok(record)
                if authorization
                    .event_role(&event.id.to_hex(), &event.pubkey.to_hex())
                    .is_some() =>
            {
                groups
                    .entry(record.record_id)
                    .or_default()
                    .push((event, record))
            }
            Ok(_) => result.diagnostics.push(format!(
                "{}: author is not an authorized workspace member",
                event.id
            )),
            Err(error) => result.diagnostics.push(format!("{}: {error}", event.id)),
        }
    }
    for (id, mut events) in groups {
        if events.len() > 4096 {
            result
                .diagnostics
                .push(format!("{id}: history exceeds 4096 revisions"));
            continue;
        }
        events.sort_by_key(|(event, record)| (record.revision, event.created_at, event.id));
        events.dedup_by_key(|(event, _)| event.id);
        let root_authors = events
            .iter()
            .filter(|(_, record)| record.revision == 0)
            .map(|(event, _)| event.pubkey)
            .collect::<std::collections::BTreeSet<_>>();
        if root_authors.len() != 1 {
            result
                .diagnostics
                .push(format!("{id}: missing or conflicting record creator"));
            continue;
        }
        let mut current: Option<DispatchRecordProjection> = None;
        for (event, record) in events {
            let author = event.pubkey.to_hex();
            let maintainer = authorization.event_role(&event.id.to_hex(), &author)
                == Some(TrackerRole::Maintainer);
            let allowed = if let Some(previous) = &current {
                let decision_change = match (&previous.record.payload, &record.payload) {
                    (
                        DispatchRecordPayload::Decision {
                            decider,
                            selected_option: old,
                            ..
                        },
                        DispatchRecordPayload::Decision {
                            selected_option: next,
                            ..
                        },
                    ) if old != next || old.is_some() => author == *decider,
                    _ => author == previous.creator || maintainer,
                };
                let automation_scope_unchanged = match (&previous.record.payload, &record.payload) {
                    (
                        DispatchRecordPayload::Automation { controls: old },
                        DispatchRecordPayload::Automation { controls: new },
                    ) => {
                        let mut scope = old.clone();
                        scope.mode = new.mode;
                        scope.expires_at = new.expires_at;
                        scope == *new
                    }
                    _ => true,
                };
                automation_scope_unchanged
                    && record.parent_event.as_ref() == Some(&previous.event_id)
                    && record.revision == previous.record.revision + 1
                    && slot(&record.payload) == slot(&previous.record.payload)
                    && event.created_at.as_secs() >= previous.created_at
                    && decision_change
            } else {
                record.revision == 0
                    && (!controlled(&record.payload) || maintainer)
                    && !matches!(&record.payload,DispatchRecordPayload::Decision{selected_option:Some(_), decider, ..} if decider != &author)
            };
            if !allowed || (controlled(&record.payload) && !maintainer) {
                result.diagnostics.push(format!(
                    "{}: unauthorized, conflicting or stale planning revision",
                    event.id
                ));
                continue;
            }
            let root_event = current
                .as_ref()
                .map(|previous| previous.root_event.clone())
                .unwrap_or_else(|| event.id.to_hex());
            let creator = current
                .as_ref()
                .map(|previous| previous.creator.clone())
                .unwrap_or_else(|| author.clone());
            let mut accepted_events = current
                .as_ref()
                .map(|previous| previous.accepted_events.clone())
                .unwrap_or_default();
            accepted_events.push(event.id.to_hex());
            current = Some(DispatchRecordProjection {
                accepted_events,
                creator,
                author,
                event_id: event.id.to_hex(),
                root_event,
                created_at: event.created_at.as_secs(),
                record,
            });
        }
        if let Some(current) = current {
            result.records.insert(id, current);
        }
    }
    result
}

/// Select active records with singleton settings resolved consistently across clients.
/// Archiving the latest setting does not resurrect an older active setting.
pub fn active_dispatch_records(
    records: &BTreeMap<Uuid, DispatchRecordProjection>,
) -> Vec<&DispatchRecordProjection> {
    let mut latest = BTreeMap::<String, &DispatchRecordProjection>::new();
    for value in records.values() {
        let key = match &value.record.payload {
            DispatchRecordPayload::Activation { .. }
            | DispatchRecordPayload::Capacity { .. }
            | DispatchRecordPayload::ProjectPlan { .. }
            | DispatchRecordPayload::Automation { .. } => slot(&value.record.payload),
            _ => value.record.record_id.to_string(),
        };
        if latest
            .get(&key)
            .is_none_or(|old| (value.created_at, &value.event_id) > (old.created_at, &old.event_id))
        {
            latest.insert(key, value);
        }
    }
    latest
        .into_values()
        .filter(|value| !value.record.archived)
        .collect()
}
