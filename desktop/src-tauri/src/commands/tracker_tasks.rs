use std::collections::BTreeMap;

use buzz_sdk_pkg::task_authority::fold_authorized_task_events;
use buzz_sdk_pkg::task_envelope::TaskEnvelopeEvent;
use buzz_sdk_pkg::tracker::IssueProjection;
use nostr::Event;
use serde_json::{json, Value};
use uuid::Uuid;

fn channel_membership<'a>(
    channel: Uuid,
    workspace: &'a Event,
    memberships: &'a [Event],
) -> Option<&'a Event> {
    let channel = channel.to_string();
    std::iter::once(workspace)
        .chain(memberships)
        .filter(|event| {
            event.kind.as_u16() == 39002
                && event.pubkey == workspace.pubkey
                && event.verify().is_ok()
                && event.tags.iter().any(|tag| {
                    let tag = tag.as_slice();
                    tag.first().map(String::as_str) == Some("d") && tag.get(1) == Some(&channel)
                })
        })
        .max_by_key(|event| (event.created_at, event.id))
}

fn member(event: &Event, pubkey: &str) -> bool {
    event.tags.iter().any(|tag| {
        let tag = tag.as_slice();
        tag.first().map(String::as_str) == Some("p")
            && tag.get(1).map(String::as_str) == Some(pubkey)
    })
}

pub(super) fn fold_linked_tasks(
    raw_events: Vec<Value>,
    memberships: &[Event],
    workspace: &Event,
    viewer: &str,
    issues: &BTreeMap<(Uuid, Uuid), IssueProjection>,
) -> (Vec<Value>, Vec<String>) {
    let mut diagnostics = Vec::new();
    let mut events = Vec::new();
    let signed: Vec<Event> = raw_events
        .iter()
        .filter_map(|raw| serde_json::from_value(raw.clone()).ok())
        .collect();
    let mut admissions = BTreeMap::new();
    let mut legacy_channels = std::collections::BTreeSet::new();
    for roster in std::iter::once(workspace).chain(memberships) {
        let channel = roster.tags.iter().find_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some("d"))
                .then(|| values.get(1)?.parse::<Uuid>().ok())
                .flatten()
        });
        let Some(channel) = channel else {
            continue;
        };
        if admissions.contains_key(&channel) {
            continue;
        }
        let Some(current) = channel_membership(channel, workspace, memberships) else {
            continue;
        };
        if !member(current, viewer) {
            continue;
        }
        let (accepted, coverage) = buzz_sdk_pkg::dispatch_admission::admitted_task_events(
            &signed,
            channel,
            current,
            nostr::Timestamp::now().as_secs(),
        );
        if coverage.unsealed > 0 {
            legacy_channels.insert(channel);
        }
        diagnostics.extend(coverage.diagnostics);
        admissions.insert(
            channel,
            accepted
                .into_iter()
                .map(|event| event.event_id)
                .collect::<std::collections::BTreeSet<_>>(),
        );
    }
    for raw in raw_events {
        if raw.get("kind").and_then(Value::as_u64) == Some(40027) {
            continue;
        }
        let parsed = serde_json::from_value::<Event>(raw)
            .map_err(|error| error.to_string())
            .and_then(|event| {
                TaskEnvelopeEvent::from_event(&event).map_err(|error| error.to_string())
            });
        let event = match parsed {
            Ok(event) => event,
            Err(error) => {
                diagnostics.push(error);
                continue;
            }
        };
        let issue = event
            .envelope
            .tracker_issue_id
            .and_then(|id| issues.values().find(|issue| issue.issue_id == id));
        let Some(issue) = issue else {
            continue;
        };
        let explicitly_linked = issue.external_refs.iter().any(|reference| {
            (reference.kind == "task" && reference.reference == event.envelope.task_id.to_string())
                || (reference.kind == "task_event"
                    && (reference.reference == event.event_id
                        || event
                            .envelope
                            .authority
                            .as_ref()
                            .and_then(|a| a.assignment_event_id.as_ref())
                            == Some(&reference.reference)))
        });
        if event.channel_id != issue.stream_id && !explicitly_linked {
            diagnostics.push(format!(
                "{}: task has no explicit link from this issue",
                event.event_id
            ));
            continue;
        }
        let membership = channel_membership(event.channel_id, workspace, memberships);
        if !membership.is_some_and(|membership| {
            member(membership, viewer)
                && admissions
                    .get(&event.channel_id)
                    .is_some_and(|ids| ids.contains(&event.event_id))
        }) {
            diagnostics.push(format!(
                "{}: execution channel membership is unavailable or does not authorize this task",
                event.event_id
            ));
            continue;
        }
        events.push(event);
    }
    let folded = fold_authorized_task_events(events);
    diagnostics.extend(
        folded
            .diagnostics
            .into_iter()
            .map(|diagnostic| format!("{}: {}", diagnostic.event_id, diagnostic.reason)),
    );
    let tasks = folded.current.into_values().map(|event| json!({
        "issueId": event.envelope.tracker_issue_id,
        "taskId": event.envelope.task_id,
        "title": event.envelope.title,
        "status": event.envelope.status,
        "assignee": event.envelope.assignee,
        "assigner": event.envelope.assigner,
        "reviewer": event.envelope.authority.as_ref().and_then(|authority| authority.reviewer.as_ref()),
        "authorPubkey": event.author_pubkey,
        "eventId": event.event_id,
        "assignmentEventId": folded.assignments.get(&event.envelope.task_id).map(|root| &root.event_id),
        "assignedAt": folded.assignments.get(&event.envelope.task_id).map(|root| root.created_at),
        "history": folded.history.get(&event.envelope.task_id).map(|history| history.iter().map(|entry| json!({
            "eventId": entry.event_id, "authorPubkey": entry.author_pubkey,
            "createdAt": entry.created_at, "status": entry.envelope.status,
            "notes": entry.envelope.notes, "blockedOn": entry.envelope.blocked_on,
            "evidence": entry.envelope.verification.evidence_ref,
        })).collect::<Vec<_>>()).unwrap_or_default(),
        "channelId": event.channel_id,
        "admissionBasis": if legacy_channels.contains(&event.channel_id) { "legacy" } else { "sealed" },
        "createdAt": event.created_at,
        "dispatchRequestId": event.envelope.dispatch_request_id,
        "blockedOn": event.envelope.blocked_on,
        "notes": event.envelope.notes,
        "deliverables": event.envelope.deliverables,
        "verification": event.envelope.verification,
    })).collect();
    (tasks, diagnostics)
}
