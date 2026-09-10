//! Nostr-native read/preparation command for clients without the Rust runtime.
//! Responses never sign or publish an edit. Mobile signs the prepared event with
//! its current identity, retains it before publication and reads canonical state.
use crate::{
    dispatch_admission::apply_dispatch_admissions,
    dispatch_record::{
        active_dispatch_records, build_dispatch_record, fold_dispatch_records,
        DispatchRecordPayload,
    },
    tracker::*,
};
use nostr::hashes::{sha256, Hash};
use nostr::{Event, EventBuilder, Kind, PublicKey, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

/// Bounded mobile issue fields. Execution assignment is a separate operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MobileIssueSet {
    /// Optional workflow status.
    pub status: Option<IssueStatus>,
    /// Optional priority.
    pub priority: Option<Priority>,
    /// Optional human owner.
    pub assignee: Option<String>,
}
/// A read-only operation; preparation returns an unsigned standard event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum DispatchClientOperation {
    /// Page current work or read one issue with its decision records.
    Snapshot {
        /// Restrict to one issue, with description and evidence links.
        issue_id: Option<Uuid>,
        /// Stable issue UUID cursor, independent of status reordering.
        after: Option<Uuid>,
        /// Include completed work when explicitly requested.
        #[serde(default)]
        include_closed: bool,
    },
    /// Prepare a current-base issue revision for local signing.
    PrepareIssue {
        /// Target issue.
        issue_id: Uuid,
        /// Frozen issue tip shown when the editor opened.
        expected_tip: String,
        /// Bounded field changes.
        set: MobileIssueSet,
        /// Explicit owner removal.
        #[serde(default)]
        clear_assignee: bool,
        /// Explicit priority removal.
        #[serde(default)]
        clear_priority: bool,
        /// Human rationale.
        reason: String,
        /// Retained intent identity.
        request_id: Uuid,
    },
    /// Prepare a decision revision for its named decision maker.
    PrepareDecision {
        /// Target decision.
        record_id: Uuid,
        /// Frozen decision revision shown before choosing.
        expected_event: String,
        /// Existing option ID.
        option: String,
        /// Visible explanation for the choice.
        rationale: String,
        /// Retained intent identity.
        request_id: Uuid,
    },
}
/// Strict versioned command payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchClientRequest {
    /// Version one.
    pub v: u8,
    /// Read or preparation operation.
    pub request: DispatchClientOperation,
}
/// Build a command for either Nostr transport; it never enters shared history.
pub fn build_dispatch_client_request(
    workspace: Uuid,
    request: &DispatchClientRequest,
) -> Result<EventBuilder, String> {
    if request.v != 1 || workspace.is_nil() {
        return Err("invalid Dispatch client scope/version".into());
    }
    let content = serde_json::to_string(request).map_err(|error| error.to_string())?;
    if content.len() > 16384 {
        return Err("Dispatch request exceeds 16 KiB".into());
    }
    Ok(EventBuilder::new(
        Kind::Custom(buzz_core::kind::KIND_DISPATCH_CLIENT as u16),
        content,
    )
    .tags([Tag::parse(["h", &workspace.to_string()]).map_err(|error| error.to_string())?]))
}
/// Compute a bounded response using the exact SDK folds used on Desktop and CLI.
/// The host must independently enforce current community/workspace read access.
pub fn dispatch_client_response(
    events: &[Event],
    workspace: Uuid,
    issuer: PublicKey,
    actor: PublicKey,
    mut authorization: TrackerAuthorization,
    request: DispatchClientRequest,
    now: u64,
) -> Result<Value, String> {
    build_dispatch_client_request(workspace, &request)?;
    let actor = actor.to_hex();
    let role = authorization
        .role(&actor)
        .ok_or("Current workspace membership is required")?;
    let admission = apply_dispatch_admissions(events, workspace, issuer, &mut authorization, now);
    let tracker = fold_tracker_events(
        events
            .iter()
            .filter_map(|event| parse_tracker_event(event, now).ok())
            .filter(|event| event.stream_id == workspace),
        &authorization,
    );
    let records = fold_dispatch_records(events.iter().cloned(), workspace, &authorization, now);
    let fingerprint = tracker
        .issues
        .values()
        .flat_map(|issue| [issue.root_event.as_str(), issue.tip_event.as_str()])
        .chain(
            records
                .records
                .values()
                .map(|record| record.event_id.as_str()),
        )
        .collect::<Vec<_>>()
        .join(":");
    let mut source_ids = events
        .iter()
        .map(|event| event.id.to_hex())
        .collect::<Vec<_>>();
    source_ids.sort();
    let fingerprint = format!("{fingerprint}:{}", source_ids.join(":"));
    let snapshot_token = sha256::Hash::hash(
        format!(
            "{workspace}:{actor}:{role:?}:{fingerprint}:{}:{}",
            admission.unsealed,
            admission.diagnostics.join(";")
        )
        .as_bytes(),
    )
    .to_string();
    match request.request {
        DispatchClientOperation::Snapshot {
            issue_id,
            after,
            include_closed,
        } => {
            if issue_id.is_some() && after.is_some() {
                return Err("A detail read cannot include a page cursor".into());
            }
            if issue_id.is_some_and(|id| !tracker.issues.contains_key(&(workspace, id))) {
                return Err("Issue is unavailable".into());
            }
            let mut page = tracker
                .issues
                .values()
                .filter(|issue| {
                    issue.stream_id == workspace
                        && issue_id.is_none_or(|id| id == issue.issue_id)
                        && after.is_none_or(|id| issue.issue_id > id)
                        && (issue_id.is_some() || include_closed || !issue.status.is_terminal())
                })
                .take(101)
                .collect::<Vec<_>>();
            let more = page.len() > 100;
            page.truncate(100);
            let next = if more {
                page.last().map(|issue| issue.issue_id)
            } else {
                None
            };
            let issues=page.iter().map(|issue|{
                let compact=issue.issue_id.simple().to_string();let mut length=8;
                while length<32 && tracker.issues.values().any(|other|other.issue_id!=issue.issue_id && other.issue_id.simple().to_string().starts_with(&compact[..length])){length+=1;}
                let accepted=tracker.activity.get(&(workspace,issue.issue_id)).into_iter().flatten().map(|item|item.event_id.clone()).collect::<Vec<_>>();
                json!({"issueId":issue.issue_id,"identifier":format!("D-{}",&compact[..length]),"title":issue.title,"body":if issue_id.is_some(){&issue.body}else{""},"rootEvent":issue.root_event,"tipEvent":issue.tip_event,"status":issue.status.to_string(),"priority":issue.priority.map(|p|p.to_string()),"assignee":issue.assignee,"labels":issue.labels,"projectId":issue.project_id,"due":issue.due,"externalRefs":if issue_id.is_some(){issue.external_refs.iter().cloned().collect::<Vec<_>>()}else{vec![]},"acceptedEvents":if issue_id.is_some(){accepted}else{vec![]}})
            }).collect::<Vec<_>>();
            let candidates = if issue_id.is_some() {
                records.records.values().collect::<Vec<_>>()
            } else {
                active_dispatch_records(&records.records)
            };
            let decisions=candidates.into_iter().filter(|value|matches!(&value.record.payload,DispatchRecordPayload::Decision{issue_id:id,selected_option,..} if issue_id.map_or(selected_option.is_none(),|wanted|wanted==*id))).collect::<Vec<_>>();
            if decisions.len() > 500 {
                return Err(
                    "Decision queue exceeds the bounded mobile view; open a specific issue".into(),
                );
            }
            Ok(
                json!({"v":1,"capabilities":{"dispatchNotice":1,"taskAuthority":3},"workspace":workspace,"snapshotAt":now,"snapshotToken":snapshot_token,"viewerRole":if role==TrackerRole::Maintainer{"maintainer"}else{"member"},"admission":{"unsealed":admission.unsealed,"sealedCount":admission.sealed.len(),"legacyBaselines":admission.legacy_baselines,"diagnostics":admission.diagnostics},"issues":issues,"decisions":decisions,"nextAfter":next,"diagnostics":tracker.diagnostics.iter().map(|value|format!("{value:?}")).chain(records.diagnostics).collect::<Vec<_>>()}),
            )
        }
        DispatchClientOperation::PrepareIssue {
            issue_id,
            expected_tip,
            set,
            clear_assignee,
            clear_priority,
            reason,
            request_id,
        } => {
            let issue = tracker
                .issues
                .get(&(workspace, issue_id))
                .ok_or("Issue is unavailable")?;
            if issue.tip_event != expected_tip {
                return Err("Issue changed; refresh and review the current version".into());
            }
            if request_id.is_nil() || reason.trim().is_empty() {
                return Err("A retained request ID and reason are required".into());
            }
            if clear_assignee && set.assignee.is_some() || clear_priority && set.priority.is_some()
            {
                return Err("A field cannot be set and cleared together".into());
            }
            if set
                .assignee
                .as_ref()
                .is_some_and(|owner| authorization.role(owner).is_none())
            {
                return Err("The owner must currently belong to this workspace".into());
            }
            if set.status.is_some_and(|status| {
                status != issue.status && (issue.status.is_terminal() || status.is_terminal())
            }) && role != TrackerRole::Maintainer
            {
                return Err(
                    "Completing, cancelling or reopening work requires a maintainer".into(),
                );
            }
            let mut clear = vec![];
            if clear_assignee {
                clear.push("assignee".into());
            }
            if clear_priority {
                clear.push("priority".into());
            }
            let revision = IssueRevision {
                v: TRACKER_VERSION,
                op: "revise".into(),
                issue_id,
                root_event: issue.root_event.clone(),
                parent_event: issue.tip_event.clone(),
                revision: issue.revision + 1,
                set: RevisionSet {
                    status: set.status,
                    priority: set.priority,
                    assignee: set.assignee,
                    ..Default::default()
                },
                clear,
                relations: Default::default(),
                external_refs: Default::default(),
                reason: Some(reason),
                request_id: Some(request_id),
                import: None,
            };
            unsigned(
                build_tracker_revision(workspace, &revision).map_err(|error| error.to_string())?,
                &actor,
                now,
            )
        }
        DispatchClientOperation::PrepareDecision {
            record_id,
            expected_event,
            option,
            rationale,
            request_id,
        } => {
            let current = records
                .records
                .get(&record_id)
                .ok_or("Decision is unavailable")?;
            if current.event_id != expected_event || current.record.archived {
                return Err("Decision changed; refresh before choosing".into());
            }
            if request_id.is_nil() {
                return Err("A retained request ID is required".into());
            }
            let mut record = current.record.clone();
            let DispatchRecordPayload::Decision {
                decider,
                selected_option,
                options,
                rationale: reason,
                ..
            } = &mut record.payload
            else {
                return Err("Record is not a decision".into());
            };
            if *decider != actor {
                return Err("Only the named decision maker can choose".into());
            }
            if selected_option.is_some() {
                return Err("This decision is already resolved".into());
            }
            if !options.iter().any(|value| value.id == option) || rationale.trim().is_empty() {
                return Err("Choose an existing option and explain why".into());
            }
            *selected_option = Some(option);
            *reason = Some(rationale);
            record.revision += 1;
            record.parent_event = Some(current.event_id.clone());
            record.request_id = request_id;
            unsigned(build_dispatch_record(workspace, &record)?, &actor, now)
        }
    }
}
fn unsigned(builder: EventBuilder, actor: &str, now: u64) -> Result<Value, String> {
    let event = builder
        .custom_created_at(Timestamp::from(now))
        .build(PublicKey::from_hex(actor).map_err(|error| error.to_string())?);
    Ok(json!({"v":1,"unsigned":event}))
}

#[cfg(test)]
mod tests;
