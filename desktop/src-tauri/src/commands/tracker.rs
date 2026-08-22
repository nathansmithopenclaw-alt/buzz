use buzz_sdk_pkg::task_envelope::{fold_task_events, TaskEnvelopeEvent};
use buzz_sdk_pkg::tracker::{
    fold_tracker_events, fold_tracker_projects_at, parse_tracker_event, IssueProjection,
    RelationType, TrackerAuthorization, TrackerRole,
};
use chrono::{DateTime, Utc};
use nostr::{Event, PublicKey};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use tauri::State;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TrackerFoldCacheKey {
    workspace_id: String,
    managed_signer: String,
}

#[derive(Clone, Debug)]
struct TrackerFoldCacheEntry {
    events: BTreeMap<String, Value>,
    task_events: BTreeMap<String, Value>,
    membership_event: Value,
}

/// Rust-owned raw event history backing incremental Dispatch folds.
///
/// The renderer may submit new signed events, but it never owns or mutates the
/// canonical projection. Every incremental result is rebuilt by the same SDK
/// fold as a cold load. Entries are cleared when `apply_workspace` changes the
/// active community so identical workspace UUIDs cannot share history.
#[derive(Default)]
pub(crate) struct TrackerFoldCache {
    entries: Mutex<HashMap<TrackerFoldCacheKey, TrackerFoldCacheEntry>>,
}

fn cache_key(workspace_id: &str, managed_signer: &str) -> Result<TrackerFoldCacheKey, String> {
    let workspace_id = workspace_id
        .parse::<Uuid>()
        .map_err(|error| format!("invalid Tracker workspace UUID: {error}"))?
        .to_string();
    let managed_signer = PublicKey::parse(managed_signer)
        .map_err(|error| format!("invalid Tracker managed signer: {error}"))?
        .to_hex();
    Ok(TrackerFoldCacheKey {
        workspace_id,
        managed_signer,
    })
}

fn raw_event_key(raw: &Value) -> String {
    raw.get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(|id| format!("id:{id}"))
        .unwrap_or_else(|| format!("raw:{raw}"))
}

fn index_raw_events(events: Vec<Value>) -> BTreeMap<String, Value> {
    events
        .into_iter()
        .map(|event| (raw_event_key(&event), event))
        .collect()
}

fn merge_raw_events(target: &mut BTreeMap<String, Value>, events: Vec<Value>) {
    for event in events {
        target.insert(raw_event_key(&event), event);
    }
}

impl TrackerFoldCache {
    pub(crate) fn clear(&self) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        entries.clear();
    }

    fn seed(
        &self,
        key: TrackerFoldCacheKey,
        events: Vec<Value>,
        task_events: Vec<Value>,
        membership_event: Value,
    ) -> Result<(), String> {
        self.entries
            .lock()
            .map_err(|error| error.to_string())?
            .insert(
                key,
                TrackerFoldCacheEntry {
                    events: index_raw_events(events),
                    task_events: index_raw_events(task_events),
                    membership_event,
                },
            );
        Ok(())
    }

    fn fold_incremental(
        &self,
        key: &TrackerFoldCacheKey,
        events: Vec<Value>,
        task_events: Vec<Value>,
        snapshot_at: u64,
    ) -> Result<Value, String> {
        let mut entries = self.entries.lock().map_err(|error| error.to_string())?;
        let cached = entries.get(key).ok_or_else(|| {
            "incremental Tracker fold has no cold-load history; refresh the workspace".to_string()
        })?;
        let mut merged = cached.clone();
        merge_raw_events(&mut merged.events, events);
        merge_raw_events(&mut merged.task_events, task_events);

        let folded = fold_tracker_context_values(
            merged.events.values().cloned().collect(),
            merged.task_events.values().cloned().collect(),
            merged.membership_event.clone(),
            &key.workspace_id,
            &key.managed_signer,
            snapshot_at,
        )?;
        entries.insert(key.clone(), merged);
        Ok(folded)
    }
}

fn authorization_from_membership(event: &Event, managed_signer: &str) -> TrackerAuthorization {
    let mut roles = vec![(event.pubkey.to_hex(), TrackerRole::Maintainer)];
    for tag in event.tags.iter().map(|tag| tag.as_slice()) {
        if tag.first().map(String::as_str) != Some("p") {
            continue;
        }
        let Some(pubkey) = tag.get(1).filter(|value| PublicKey::parse(value).is_ok()) else {
            continue;
        };
        let role = match tag.get(3).map(String::as_str).unwrap_or("member") {
            "owner" | "admin" | "bot" => TrackerRole::Maintainer,
            _ => TrackerRole::Member,
        };
        roles.push((pubkey.clone(), role));
    }
    // TrackerAuthorization is last-write-wins for duplicate pubkeys. Match the
    // CLI's auth-tag-owner semantics by applying the Desktop identity override
    // after membership rows so a `member` tag cannot downgrade the owner.
    roles.push((managed_signer.to_string(), TrackerRole::Maintainer));
    TrackerAuthorization::new(roles)
}

fn project_signer_from_membership(event: &Event) -> Result<String, String> {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find_map(|tag| {
            if tag.first().map(String::as_str) != Some("p")
                || tag.get(3).map(String::as_str) != Some("owner")
            {
                return None;
            }
            tag.get(1)
                .and_then(|pubkey| PublicKey::parse(pubkey).ok())
                .map(|pubkey| pubkey.to_hex())
        })
        .ok_or_else(|| "membership snapshot has no valid workspace owner row".to_string())
}

fn issue_json(issue: &IssueProjection) -> Value {
    let relations = issue
        .relations
        .iter()
        .map(|relation| {
            json!({
                "kind": match relation.kind {
                    RelationType::Blocks => "blocks",
                    RelationType::DuplicateOf => "duplicate_of",
                    RelationType::Parent => "parent",
                },
                "targetIssueId": relation.target_issue_id,
                "targetRootEvent": relation.target_root_event,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "streamId": issue.stream_id,
        "issueId": issue.issue_id,
        "rootEvent": issue.root_event,
        "rootCreatedAt": issue.root_created_at,
        "tipEvent": issue.tip_event,
        "tipCreatedAt": issue.tip_created_at,
        "revision": issue.revision,
        "enteredStatusAt": issue.entered_status_at,
        "identifier": issue.identifier,
        "title": issue.title,
        "body": issue.body,
        "status": issue.status.to_string(),
        "priority": issue.priority.map(|value| value.to_string()),
        "assignee": issue.assignee,
        "projectId": issue.project_id,
        "due": issue.due,
        "estimateMinutes": issue.estimate_minutes,
        "homeChannelId": issue.home_channel_id,
        "workflow": issue.workflow,
        "labels": issue.labels,
        "externalRefs": issue.external_refs,
        "relations": relations,
    })
}

fn issue_changes(before: &IssueProjection, after: &IssueProjection) -> Vec<Value> {
    let before = issue_json(before);
    let after = issue_json(after);
    [
        "identifier",
        "title",
        "body",
        "status",
        "priority",
        "assignee",
        "projectId",
        "due",
        "estimateMinutes",
        "homeChannelId",
        "workflow",
        "labels",
        "externalRefs",
        "relations",
    ]
    .into_iter()
    .filter_map(|field| {
        let old = before.get(field)?;
        let new = after.get(field)?;
        (old != new).then(|| json!({"field": field, "before": old, "after": new}))
    })
    .collect()
}

fn fold_tracker_context_values(
    raw_events: Vec<Value>,
    raw_task_events: Vec<Value>,
    membership_event: Value,
    workspace_id: &str,
    managed_signer: &str,
    snapshot_at: u64,
) -> Result<Value, String> {
    let workspace_id = workspace_id
        .parse::<Uuid>()
        .map_err(|error| format!("invalid Tracker workspace UUID: {error}"))?;
    let membership_event: Event = serde_json::from_value(membership_event)
        .map_err(|error| format!("invalid membership snapshot: {error}"))?;
    membership_event
        .verify()
        .map_err(|error| format!("invalid membership snapshot signature: {error}"))?;
    let workspace_tag = workspace_id.to_string();
    if membership_event.kind.as_u16() != 39002
        || !membership_event.tags.iter().any(|tag| {
            let tag = tag.as_slice();
            tag.first().map(String::as_str) == Some("d")
                && tag.get(1).map(String::as_str) == Some(workspace_tag.as_str())
        })
    {
        return Err("membership snapshot does not identify the Tracker workspace".to_string());
    }
    let managed_signer = PublicKey::parse(managed_signer)
        .map_err(|error| format!("invalid Tracker managed signer: {error}"))?
        .to_hex();
    let project_signer = project_signer_from_membership(&membership_event)?;
    let authorization = authorization_from_membership(&membership_event, &managed_signer);
    let mut signed_events = Vec::new();
    let mut parsed_tracker_events = Vec::new();
    let mut diagnostics = Vec::new();

    for raw in raw_events {
        let event: Event = match serde_json::from_value(raw) {
            Ok(event) => event,
            Err(error) => {
                diagnostics.push(json!({"kind": "InvalidEvent", "message": error.to_string()}));
                continue;
            }
        };
        if matches!(event.kind.as_u16() as u32, 40020..=40022) {
            match parse_tracker_event(&event, snapshot_at) {
                Ok(event) if event.stream_id == workspace_id => parsed_tracker_events.push(event),
                Ok(_) => {}
                Err(error) => diagnostics.push(json!({
                    "kind": "InvalidEvent",
                    "message": format!("{}: {error}", event.id.to_hex()),
                })),
            }
        }
        signed_events.push(event);
    }

    let folded = fold_tracker_events(parsed_tracker_events, &authorization);
    let projects =
        fold_tracker_projects_at(signed_events, workspace_id, &project_signer, snapshot_at);
    diagnostics.extend(folded.diagnostics.iter().map(|diagnostic| {
        json!({"kind": format!("{diagnostic:?}").split_whitespace().next().unwrap_or("FoldDiagnostic"), "message": format!("{diagnostic:?}")})
    }));
    diagnostics.extend(projects.diagnostics.iter().map(
        |diagnostic| json!({"kind": "ProjectDiagnostic", "message": format!("{diagnostic:?}")}),
    ));

    let issues = folded.issues.values().map(issue_json).collect::<Vec<_>>();
    let activity = folded
        .activity
        .iter()
        .flat_map(|((_, issue_id), entries)| {
            entries.iter().map(move |entry| {
                json!({
                    "issueId": issue_id,
                    "eventId": entry.event_id,
                    "authorPubkey": entry.author_pubkey,
                    "createdAt": entry.created_at,
                    "revision": entry.revision,
                    "reason": entry.reason,
                    "requestId": entry.request_id,
                    "import": entry.import,
                    "changes": entry.before.as_ref().map(|before| issue_changes(before, &entry.after)).unwrap_or_default(),
                })
            })
        })
        .collect::<Vec<_>>();
    let comments = folded
        .comments
        .iter()
        .flat_map(|((_, issue_id), entries)| {
            entries.iter().map(move |comment| {
                json!({
                    "issueId": issue_id,
                    "eventId": comment.event_id,
                    "authorPubkey": comment.author_pubkey,
                    "createdAt": comment.created_at,
                    "rootEvent": comment.root_event,
                    "parentEvent": comment.parent_event,
                    "body": comment.body,
                    "requestId": comment.request_id,
                    "import": comment.import,
                })
            })
        })
        .collect::<Vec<_>>();
    let mut task_diagnostics = Vec::new();
    let mut task_events = Vec::new();
    for raw in raw_task_events {
        let event: Event = match serde_json::from_value(raw) {
            Ok(event) => event,
            Err(error) => {
                task_diagnostics.push(error.to_string());
                continue;
            }
        };
        if let Err(error) = event.verify() {
            task_diagnostics.push(format!("{}: {error}", event.id.to_hex()));
            continue;
        }
        match TaskEnvelopeEvent::from_event(&event) {
            Ok(event) if event.channel_id != workspace_id => {
                task_diagnostics.push(format!(
                    "{}: task channel is outside the Tracker workspace",
                    event.event_id
                ));
            }
            Ok(event) if authorization.role(&event.author_pubkey).is_none() => {
                task_diagnostics.push(format!(
                    "{}: task author is not a workspace member",
                    event.event_id
                ));
            }
            Ok(event) => task_events.push(event),
            Err(error) => task_diagnostics.push(format!("{}: {error}", event.id.to_hex())),
        }
    }
    let linked_tasks = fold_task_events(task_events)
        .into_values()
        .filter_map(|event| {
            let issue_id = event.envelope.tracker_issue_id?;
            folded
                .issues
                .keys()
                .any(|(_, candidate)| *candidate == issue_id)
                .then(|| {
                    json!({
                        "issueId": issue_id,
                        "taskId": event.envelope.task_id,
                        "title": event.envelope.title,
                        "status": event.envelope.status,
                        "assignee": event.envelope.assignee,
                        "eventId": event.event_id,
                        "channelId": event.channel_id,
                        "createdAt": event.created_at,
                        "dispatchRequestId": event.envelope.dispatch_request_id,
                    })
                })
        })
        .collect::<Vec<_>>();
    let projects = projects
        .projects
        .values()
        .map(|project| {
            json!({
                "projectId": project.project_id,
                "slug": project.slug,
                "name": project.name,
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "issues": issues,
        "activity": activity,
        "comments": comments,
        "linkedTasks": linked_tasks,
        "projects": projects,
        "diagnostics": diagnostics,
        "taskDiagnostics": task_diagnostics,
        "viewerRole": match authorization.role(&managed_signer) {
            Some(TrackerRole::Maintainer) => "maintainer",
            Some(TrackerRole::Member) => "member",
            None => "none",
        },
        "snapshotAt": DateTime::<Utc>::from_timestamp(snapshot_at as i64, 0)
            .ok_or_else(|| "Tracker snapshot timestamp is out of range".to_string())?
            .to_rfc3339(),
    }))
}

#[tauri::command]
pub fn fold_tracker_context(
    events: Vec<Value>,
    task_events: Vec<Value>,
    membership_event: Value,
    workspace_id: String,
    managed_signer: String,
    snapshot_at: u64,
    cache: State<'_, TrackerFoldCache>,
) -> Result<Value, String> {
    let key = cache_key(&workspace_id, &managed_signer)?;
    let folded = fold_tracker_context_values(
        events.clone(),
        task_events.clone(),
        membership_event.clone(),
        &workspace_id,
        &managed_signer,
        snapshot_at,
    )?;
    cache.seed(key, events, task_events, membership_event)?;
    Ok(folded)
}

#[tauri::command]
pub fn fold_tracker_context_incremental(
    events: Vec<Value>,
    task_events: Vec<Value>,
    workspace_id: String,
    managed_signer: String,
    snapshot_at: u64,
    cache: State<'_, TrackerFoldCache>,
) -> Result<Value, String> {
    let key = cache_key(&workspace_id, &managed_signer)?;
    cache.fold_incremental(&key, events, task_events, snapshot_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_sdk_pkg::build_project_with_tags;
    use buzz_sdk_pkg::task_envelope::{
        build_task_message, TaskDeliverable, TaskEnvelope, TaskStatus, TaskVerification,
        TASK_ENVELOPE_VERSION,
    };
    use buzz_sdk_pkg::tracker::{
        build_tracker_comment, build_tracker_issue, build_tracker_revision, IssueComment,
        IssueRevision, IssueRoot, IssueStatus, Priority, RevisionSet, SetOperations,
        TRACKER_VERSION,
    };
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn membership(workspace_id: Uuid, member: &Keys, owner: &Keys) -> Event {
        let relay = Keys::generate();
        EventBuilder::new(Kind::Custom(39002), "")
            .tags([
                Tag::parse(["d", workspace_id.to_string().as_str()]).expect("workspace tag"),
                Tag::parse(["p", member.public_key().to_hex().as_str(), "", "member"])
                    .expect("member tag"),
                Tag::parse(["p", owner.public_key().to_hex().as_str(), "", "owner"])
                    .expect("owner tag"),
            ])
            .sign_with_keys(&relay)
            .expect("signed membership")
    }

    #[test]
    fn fold_tracker_context_matches_the_sdk_issue_and_project_projections() {
        let workspace_id = Uuid::new_v4();
        let issue_id = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let member = Keys::generate();
        let owner = Keys::generate();
        let membership = membership(workspace_id, &member, &owner);
        let root = IssueRoot {
            v: 1,
            op: "create".to_string(),
            issue_id,
            identifier: "DSP-123".to_string(),
            title: "Canonical fold fixture".to_string(),
            body: "Fixture body".to_string(),
            // A terminal root requires Maintainer authorization. The fixture's
            // managed signer is also tagged as a member, proving the final
            // identity override wins over the membership row.
            status: IssueStatus::Done,
            priority: Some(Priority::P1),
            assignee: Some(member.public_key().to_hex()),
            project_id: None,
            due: None,
            estimate_minutes: Some(30),
            home_channel_id: None,
            source_event_id: None,
            workflow: "default".to_string(),
            labels: vec!["desktop".to_string()],
            external_refs: Vec::new(),
            request_id: None,
            import: None,
        };
        let event = build_tracker_issue(workspace_id, &root)
            .expect("root builder")
            .custom_created_at(nostr::Timestamp::from(100))
            .sign_with_keys(&member)
            .expect("signed root");
        let revision = build_tracker_revision(
            workspace_id,
            &IssueRevision {
                v: TRACKER_VERSION,
                op: "revise".to_string(),
                issue_id,
                root_event: event.id.to_hex(),
                parent_event: event.id.to_hex(),
                revision: 1,
                set: RevisionSet {
                    status: Some(IssueStatus::InProgress),
                    ..RevisionSet::default()
                },
                clear: Vec::new(),
                relations: SetOperations::default(),
                external_refs: SetOperations::default(),
                reason: None,
                request_id: None,
                import: None,
            },
        )
        .expect("revision builder")
        .custom_created_at(nostr::Timestamp::from(101))
        .sign_with_keys(&member)
        .expect("signed revision");
        let comment = build_tracker_comment(
            workspace_id,
            &event.id.to_hex(),
            None,
            &IssueComment {
                v: TRACKER_VERSION,
                issue_id,
                body: "Canonical comment".to_string(),
                request_id: None,
                import: None,
            },
            &[],
        )
        .expect("comment builder")
        .custom_created_at(nostr::Timestamp::from(102))
        .sign_with_keys(&member)
        .expect("signed comment");
        let project = build_project_with_tags(
            "",
            vec![
                Tag::parse(["d", "dispatch"]).expect("project slug"),
                Tag::parse(["name", "Dispatch"]).expect("project name"),
                Tag::parse(["buzz-project-id", project_id.to_string().as_str()])
                    .expect("project id"),
                Tag::parse(["buzz-workspace", workspace_id.to_string().as_str()])
                    .expect("project workspace"),
            ],
        )
        .expect("project builder")
        .custom_created_at(nostr::Timestamp::from(100))
        .sign_with_keys(&owner)
        .expect("signed project");
        let task_id = Uuid::new_v4();
        let task = build_task_message(
            workspace_id,
            "Dispatch fixture",
            &TaskEnvelope {
                v: TASK_ENVELOPE_VERSION,
                task_id,
                status: TaskStatus::PickedUp,
                assignee: member.public_key().to_hex(),
                assigner: member.public_key().to_hex(),
                title: "Canonical linked task".to_string(),
                deliverables: Vec::<TaskDeliverable>::new(),
                verification: TaskVerification {
                    method: "Fixture verification".to_string(),
                    evidence_ref: None,
                },
                blocked_on: None,
                notes: None,
                tracker_issue_id: Some(issue_id),
                dispatch_request_id: Some(Uuid::new_v4()),
            },
            None,
            &[],
        )
        .expect("task builder")
        .custom_created_at(nostr::Timestamp::from(103))
        .sign_with_keys(&member)
        .expect("signed task");
        let outsider = Keys::generate();
        let outsider_task = build_task_message(
            workspace_id,
            "Untrusted replacement",
            &TaskEnvelope {
                v: TASK_ENVELOPE_VERSION,
                task_id,
                status: TaskStatus::Done,
                assignee: outsider.public_key().to_hex(),
                assigner: outsider.public_key().to_hex(),
                title: "Untrusted linked task".to_string(),
                deliverables: Vec::<TaskDeliverable>::new(),
                verification: TaskVerification {
                    method: "Untrusted verification".to_string(),
                    evidence_ref: None,
                },
                blocked_on: None,
                notes: None,
                tracker_issue_id: Some(issue_id),
                dispatch_request_id: None,
            },
            None,
            &[],
        )
        .expect("outsider task builder")
        .custom_created_at(nostr::Timestamp::from(104))
        .sign_with_keys(&outsider)
        .expect("signed outsider task");
        let foreign_task = build_task_message(
            Uuid::new_v4(),
            "Foreign workspace task",
            &TaskEnvelope {
                v: TASK_ENVELOPE_VERSION,
                task_id: Uuid::new_v4(),
                status: TaskStatus::PickedUp,
                assignee: member.public_key().to_hex(),
                assigner: member.public_key().to_hex(),
                title: "Foreign linked task".to_string(),
                deliverables: Vec::<TaskDeliverable>::new(),
                verification: TaskVerification {
                    method: "Foreign verification".to_string(),
                    evidence_ref: None,
                },
                blocked_on: None,
                notes: None,
                tracker_issue_id: Some(issue_id),
                dispatch_request_id: None,
            },
            None,
            &[],
        )
        .expect("foreign task builder")
        .custom_created_at(nostr::Timestamp::from(104))
        .sign_with_keys(&member)
        .expect("signed foreign task");
        let snapshot_at = 104;
        let managed_signer = member.public_key().to_hex();

        let actual = fold_tracker_context_values(
            vec![
                serde_json::to_value(&event).expect("root JSON"),
                serde_json::to_value(&revision).expect("revision JSON"),
                serde_json::to_value(&comment).expect("comment JSON"),
                serde_json::to_value(&project).expect("project JSON"),
            ],
            vec![
                serde_json::to_value(&task).expect("task JSON"),
                serde_json::to_value(&outsider_task).expect("outsider task JSON"),
                serde_json::to_value(&foreign_task).expect("foreign task JSON"),
            ],
            serde_json::to_value(&membership).expect("membership JSON"),
            &workspace_id.to_string(),
            &managed_signer,
            snapshot_at,
        )
        .expect("fold context");
        let parsed = [event.clone(), revision.clone(), comment.clone()]
            .into_iter()
            .map(|event| parse_tracker_event(&event, snapshot_at).expect("SDK parse"));
        let expected = fold_tracker_events(
            parsed,
            &authorization_from_membership(&membership, &managed_signer),
        );
        let expected_projects = fold_tracker_projects_at(
            [project],
            workspace_id,
            &owner.public_key().to_hex(),
            snapshot_at,
        );
        let expected_issue = expected
            .issues
            .get(&(workspace_id, issue_id))
            .expect("SDK projection");
        let actual_issue = actual["issues"]
            .as_array()
            .and_then(|issues| issues.first())
            .expect("command projection");

        assert_eq!(actual_issue["issueId"], issue_id.to_string());
        assert_eq!(actual_issue["tipEvent"], expected_issue.tip_event);
        assert_eq!(actual_issue["revision"], 1);
        assert_eq!(actual_issue["enteredStatusAt"], 101);
        assert_eq!(actual_issue["identifier"], expected_issue.identifier);
        assert_eq!(actual_issue["title"], expected_issue.title);
        assert_eq!(actual_issue["status"], expected_issue.status.to_string());
        assert_eq!(actual_issue["priority"], "P1");
        assert_eq!(actual_issue["labels"], json!(["desktop"]));
        assert_eq!(actual["projects"][0]["projectId"], project_id.to_string());
        assert_eq!(actual["projects"][0]["slug"], "dispatch");
        assert_eq!(
            actual["projects"][0]["name"],
            expected_projects.projects[&project_id].name
        );
        assert_eq!(actual["activity"].as_array().map(Vec::len), Some(2));
        assert_eq!(actual["activity"][1]["changes"][0]["field"], "status");
        assert_eq!(actual["comments"][0]["body"], "Canonical comment");
        assert_eq!(actual["linkedTasks"][0]["taskId"], task_id.to_string());
        assert_eq!(actual["linkedTasks"][0]["title"], "Canonical linked task");
        assert_eq!(actual["linkedTasks"][0]["status"], "picked_up");
        assert_eq!(actual["viewerRole"], "maintainer");
        let task_diagnostics = actual["taskDiagnostics"]
            .as_array()
            .expect("task diagnostics");
        assert_eq!(task_diagnostics.len(), 2);
        assert!(task_diagnostics.iter().any(|message| message
            .as_str()
            .is_some_and(|message| message.contains("not a workspace member"))));
        assert!(task_diagnostics.iter().any(|message| message
            .as_str()
            .is_some_and(|message| message.contains("outside the Tracker workspace"))));
        assert_eq!(actual["diagnostics"], json!([]));
    }

    #[test]
    fn fold_tracker_context_uses_the_workspace_owner_for_project_heads() {
        let workspace_id = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let member_project_id = Uuid::new_v4();
        let member = Keys::generate();
        let owner = Keys::generate();
        let membership = membership(workspace_id, &member, &owner);
        let project = build_project_with_tags(
            "",
            vec![
                Tag::parse(["d", "dispatch"]).expect("project slug"),
                Tag::parse(["name", "Dispatch"]).expect("project name"),
                Tag::parse(["buzz-project-id", project_id.to_string().as_str()])
                    .expect("project id"),
                Tag::parse(["buzz-workspace", workspace_id.to_string().as_str()])
                    .expect("project workspace"),
            ],
        )
        .expect("project builder")
        .sign_with_keys(&owner)
        .expect("signed project");
        let member_project = build_project_with_tags(
            "",
            vec![
                Tag::parse(["d", "member-project"]).expect("project slug"),
                Tag::parse(["name", "Member project"]).expect("project name"),
                Tag::parse(["buzz-project-id", member_project_id.to_string().as_str()])
                    .expect("project id"),
                Tag::parse(["buzz-workspace", workspace_id.to_string().as_str()])
                    .expect("project workspace"),
            ],
        )
        .expect("project builder")
        .sign_with_keys(&member)
        .expect("signed member project");
        let snapshot_at = project
            .created_at
            .as_secs()
            .max(member_project.created_at.as_secs())
            + 1;

        let actual = fold_tracker_context_values(
            vec![
                serde_json::to_value(&project).expect("project JSON"),
                serde_json::to_value(&member_project).expect("member project JSON"),
            ],
            Vec::new(),
            serde_json::to_value(&membership).expect("membership JSON"),
            &workspace_id.to_string(),
            &member.public_key().to_hex(),
            snapshot_at,
        )
        .expect("fold context");

        assert_eq!(actual["projects"].as_array().map(Vec::len), Some(1));
        assert_eq!(actual["projects"][0]["projectId"], project_id.to_string());
        assert_eq!(actual["projects"][0]["name"], "Dispatch");
        let diagnostics = actual["diagnostics"].as_array().expect("diagnostics");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0]["kind"], "ProjectDiagnostic");
        assert!(diagnostics[0]["message"]
            .as_str()
            .is_some_and(|message| message.contains("member-project")));
    }

    #[test]
    fn incremental_fold_merges_into_the_cached_sdk_history() {
        let workspace_id = Uuid::new_v4();
        let issue_id = Uuid::new_v4();
        let member = Keys::generate();
        let owner = Keys::generate();
        let membership = membership(workspace_id, &member, &owner);
        let root = build_tracker_issue(
            workspace_id,
            &IssueRoot {
                v: TRACKER_VERSION,
                op: "create".to_string(),
                issue_id,
                identifier: "D-incremental".to_string(),
                title: "Cold title".to_string(),
                body: String::new(),
                status: IssueStatus::Backlog,
                priority: None,
                assignee: None,
                project_id: None,
                due: None,
                estimate_minutes: None,
                home_channel_id: None,
                source_event_id: None,
                workflow: "default".to_string(),
                labels: Vec::new(),
                external_refs: Vec::new(),
                request_id: None,
                import: None,
            },
        )
        .expect("root builder")
        .custom_created_at(nostr::Timestamp::from(100))
        .sign_with_keys(&member)
        .expect("signed root");
        let revision = build_tracker_revision(
            workspace_id,
            &IssueRevision {
                v: TRACKER_VERSION,
                op: "revise".to_string(),
                issue_id,
                root_event: root.id.to_hex(),
                parent_event: root.id.to_hex(),
                revision: 1,
                set: RevisionSet {
                    title: Some("Live title".to_string()),
                    status: Some(IssueStatus::InProgress),
                    ..RevisionSet::default()
                },
                clear: Vec::new(),
                relations: SetOperations::default(),
                external_refs: SetOperations::default(),
                reason: None,
                request_id: None,
                import: None,
            },
        )
        .expect("revision builder")
        .custom_created_at(nostr::Timestamp::from(101))
        .sign_with_keys(&member)
        .expect("signed revision");
        let key =
            cache_key(&workspace_id.to_string(), &member.public_key().to_hex()).expect("cache key");
        let cache = TrackerFoldCache::default();
        cache
            .seed(
                key.clone(),
                vec![serde_json::to_value(root).expect("root JSON")],
                Vec::new(),
                serde_json::to_value(membership).expect("membership JSON"),
            )
            .expect("seed cache");

        let actual = cache
            .fold_incremental(
                &key,
                vec![serde_json::to_value(revision).expect("revision JSON")],
                Vec::new(),
                102,
            )
            .expect("incremental fold");

        assert_eq!(actual["issues"].as_array().map(Vec::len), Some(1));
        assert_eq!(actual["issues"][0]["revision"], 1);
        assert_eq!(actual["issues"][0]["title"], "Live title");
        assert_eq!(actual["issues"][0]["status"], "in_progress");
        assert_eq!(actual["activity"].as_array().map(Vec::len), Some(2));
        assert!(actual["activity"]
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| entry["revision"] == 1)));
    }

    #[test]
    fn fold_tracker_context_rejects_a_membership_snapshot_for_another_workspace() {
        let requested = Uuid::new_v4();
        let membership = membership(Uuid::new_v4(), &Keys::generate(), &Keys::generate());
        let error = fold_tracker_context_values(
            Vec::new(),
            Vec::new(),
            serde_json::to_value(membership).expect("membership JSON"),
            &requested.to_string(),
            &Keys::generate().public_key().to_hex(),
            1,
        )
        .expect_err("workspace mismatch must fail");

        assert_eq!(
            error,
            "membership snapshot does not identify the Tracker workspace"
        );
    }
}
