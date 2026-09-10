//! Pure planning for explicit, bounded workspace approvals.
use crate::{
    config::ThresholdAction,
    projection::ProjectionStore,
    rules::{IssueRef, RuleAction},
};
use buzz_sdk::{
    dispatch_automation::AutomationControls,
    dispatch_record::{active_dispatch_records, DispatchRecordFold, DispatchRecordPayload},
    task_authority::TaskAuthorityFold,
    task_envelope::TaskStatus,
    tracker::{route_request_id, threshold_request_id, IssueStatus, RelationType, TrackerFold},
};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// A candidate and its deterministic explanation, without producing an effect.
#[derive(Debug)]
pub struct Candidate {
    /// Existing issue being considered.
    pub issue_id: Uuid,
    /// Why the action is ready or held.
    pub explanation: String,
    /// Ready effect; absent when input or capacity prevents safe execution.
    pub action: Option<RuleAction>,
}
/// Plan only the approval's named issues, with explicit activation, dependencies,
/// review, capability and aggregate capacity checks. Capacity is consumed while
/// planning so two candidates cannot both spend the last executor slot.
pub fn plan(
    controls: &AutomationControls,
    workspace: Uuid,
    fold: &TrackerFold,
    records: &DispatchRecordFold,
    tasks: &TaskAuthorityFold,
    store: &ProjectionStore,
    now: u64,
) -> Vec<Candidate> {
    let active = active_dispatch_records(&records.records);
    let activated: BTreeSet<Uuid> = active
        .iter()
        .filter_map(|value| match value.record.payload {
            DispatchRecordPayload::Activation {
                issue_id,
                active: true,
                ..
            } => Some(issue_id),
            _ => None,
        })
        .collect();
    let imported = store.imported_issue_ids();
    let mut used = BTreeMap::<String, usize>::new();
    for task in tasks.current.values() {
        if matches!(
            task.envelope.status,
            TaskStatus::Assigned | TaskStatus::PickedUp | TaskStatus::Blocked
        ) {
            *used.entry(task.envelope.assignee.clone()).or_default() += 1;
        }
    }
    let mut candidates = Vec::new();
    for id in &controls.issue_ids {
        let mut held = |explanation: &str| {
            candidates.push(Candidate {
                issue_id: *id,
                explanation: explanation.into(),
                action: None,
            })
        };
        let Some(issue) = fold.issues.get(&(workspace, *id)) else {
            held("Issue is unavailable in the current accepted workspace");
            continue;
        };
        if issue.status.is_terminal() {
            held("Issue is already closed");
            continue;
        }
        if imported.contains(id) && !activated.contains(id) {
            held("Imported work remains parked until a maintainer activates it");
            continue;
        }
        let reference = IssueRef {
            issue_id: *id,
            root_event: issue.root_event.clone(),
            identifier: issue.identifier.clone(),
            title: issue.title.clone(),
        };
        let route = controls
            .routing
            .iter()
            .find(|route| issue.labels.contains(&route.label));
        if let Some(route) = route {
            if issue.status != IssueStatus::Todo {
                held("Accept this work into Todo before routing");
                continue;
            }
            if fold.issues.values().any(|source| {
                !source.status.is_terminal()
                    && source.relations.iter().any(|relation| {
                        relation.kind == RelationType::Blocks
                            && relation.target_issue_id == *id
                            && relation.target_root_event == issue.root_event
                    })
            }) {
                held("An unfinished dependency blocks this issue");
                continue;
            }
            if tasks.current.values().any(|task| {
                task.envelope.tracker_issue_id == Some(*id)
                    && !matches!(
                        task.envelope.status,
                        TaskStatus::Failed | TaskStatus::Verified
                    )
            }) {
                held("An existing attempt is running or awaiting independent review");
                continue;
            }
            if issue.external_refs.iter().any(|reference| {
                reference.kind == "task"
                    && Uuid::parse_str(&reference.reference)
                        .ok()
                        .is_none_or(|id| !tasks.current.contains_key(&id))
            }) {
                held("A linked attempt is unavailable; refresh its history first");
                continue;
            }
            let Some(owner) = issue.assignee.as_ref() else {
                held("Choose an accountable owner and reviewer before routing");
                continue;
            };
            if owner == &route.executor {
                held("The reviewer must be a different person from the executor");
                continue;
            }
            let capacity = active.iter().find_map(|value| match &value.record.payload {
                DispatchRecordPayload::Capacity {
                    executor,
                    max_active,
                    capabilities,
                    paused,
                } if executor == &route.executor => Some((*max_active, capabilities, *paused)),
                _ => None,
            });
            let Some((maximum, capabilities, paused)) = capacity else {
                held("Executor capacity has not been configured");
                continue;
            };
            if paused {
                held("Executor is paused");
                continue;
            }
            if !capabilities.contains(&route.label) {
                held("Executor has not been enabled for this capability");
                continue;
            }
            let occupied = used.entry(route.executor.clone()).or_default();
            if *occupied >= maximum as usize {
                held("Executor is at capacity");
                continue;
            }
            let request_id = route_request_id(*id, &route.label);
            if store.request_fired(&request_id) {
                held("This issue and routing label were already dispatched");
                continue;
            }
            *occupied += 1;
            candidates.push(Candidate {
                issue_id: *id,
                explanation: format!(
                    "Matches {} · {}/{} slots reserved · owner remains accountable",
                    route.label, occupied, maximum
                ),
                action: Some(RuleAction::RouteAssign {
                    issue: reference,
                    label: route.label.clone(),
                    owner: route.executor.clone(),
                    channel_id: route.channel,
                    request_id,
                }),
            });
        } else if let Some(window) = controls.unassigned_after_secs.filter(|window| {
            issue.assignee.is_none() && now >= issue.entered_status_at.saturating_add(*window)
        }) {
            let threshold_id = format!("unowned_{window}");
            let request_id =
                threshold_request_id(*id, &threshold_id, issue.entered_status_at / window);
            if store.request_fired(&request_id) {
                held("The notice for this unowned interval was already delivered");
                continue;
            }
            candidates.push(Candidate {
                issue_id: *id,
                explanation: "Unowned work passed the explicitly configured waiting period".into(),
                action: Some(RuleAction::Escalate {
                    issue: reference,
                    threshold_id,
                    channel_id: controls.notification_channel,
                    action: ThresholdAction::DecisionSurface,
                    request_id,
                }),
            });
        } else if controls.notify_triage
            && issue.status == IssueStatus::Triage
            && issue.assignee.is_none()
        {
            let request_id = Uuid::new_v5(
                &buzz_sdk::tracker::DISPATCH_REQUEST_NAMESPACE,
                format!("triage:{id}:{}", issue.entered_status_at).as_bytes(),
            );
            if store.request_fired(&request_id) {
                held("This triage entry has already been announced");
                continue;
            }
            candidates.push(Candidate {
                issue_id: *id,
                explanation: "A new unowned triage entry needs human attention".into(),
                action: Some(RuleAction::NotifyTriage {
                    issue: reference,
                    channel_id: controls.notification_channel,
                    request_id,
                }),
            });
        } else {
            held("No enabled rule currently applies");
        }
    }
    candidates
}
/// Stable action identity used by both the journal and the total rollout budget.
pub fn request_id(action: &RuleAction) -> Uuid {
    match action {
        RuleAction::NotifyTriage { request_id, .. }
        | RuleAction::RouteAssign { request_id, .. }
        | RuleAction::Escalate { request_id, .. } => *request_id,
    }
}
