use std::collections::BTreeMap;
use std::collections::HashSet;
use std::str::FromStr;

use buzz_sdk::task_envelope::{
    fold_task_events, TaskDeliverable, TaskDeliverableType, TaskEnvelope, TaskEnvelopeEvent,
    TaskStatus, TaskVerification, TASK_DISCOVERY_HASHTAG, TASK_ENVELOPE_VERSION,
};
use nostr::{Event, PublicKey};
use serde_json::json;
use uuid::Uuid;

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{parse_uuid, read_or_stdin};
use crate::TasksCmd;

fn sdk_error(context: &str, error: impl std::fmt::Display) -> CliError {
    CliError::Usage(format!("{context}: {error}"))
}

fn parse_status(value: &str, allow_assigned: bool) -> Result<TaskStatus, CliError> {
    let status =
        TaskStatus::from_str(value).map_err(|error| sdk_error("invalid --status", error))?;
    if !allow_assigned && status == TaskStatus::Assigned {
        return Err(CliError::Usage(
            "tasks update cannot transition to assigned".into(),
        ));
    }
    Ok(status)
}

fn parse_deliverable(value: &str) -> Result<TaskDeliverable, CliError> {
    let (kind, reference) = value.split_once('=').ok_or_else(|| {
        CliError::Usage(format!("--deliverable must be type=ref (got {value:?})"))
    })?;
    let kind = TaskDeliverableType::from_str(kind)
        .map_err(|error| sdk_error("invalid --deliverable", error))?;
    if reference.trim().is_empty() {
        return Err(CliError::Usage(
            "--deliverable reference must not be empty".into(),
        ));
    }
    Ok(TaskDeliverable {
        kind,
        reference: reference.trim().to_string(),
    })
}

fn parse_deliverables(values: &[String]) -> Result<Vec<TaskDeliverable>, CliError> {
    values
        .iter()
        .map(|value| parse_deliverable(value))
        .collect()
}

fn parse_task_events(values: Vec<serde_json::Value>) -> Vec<TaskEnvelopeEvent> {
    values
        .into_iter()
        .filter_map(|value| serde_json::from_value::<Event>(value).ok())
        .filter_map(|event| TaskEnvelopeEvent::from_event(&event).ok())
        .collect()
}

async fn query_task_events(
    client: &BuzzClient,
    channel: Option<Uuid>,
) -> Result<Vec<TaskEnvelopeEvent>, CliError> {
    let mut filter = json!({
        "kinds": [9],
        "#t": [TASK_DISCOVERY_HASHTAG],
    });
    if let Some(channel) = channel {
        filter["#h"] = json!([channel.to_string()]);
    }
    Ok(parse_task_events(client.query_all(filter).await?))
}

fn member_pubkeys(event: &serde_json::Value) -> Vec<String> {
    event
        .get("tags")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tag| tag.as_array())
        .filter(|tag| tag.first().and_then(serde_json::Value::as_str) == Some("p"))
        .filter_map(|tag| tag.get(1).and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect()
}

async fn channel_members(client: &BuzzClient, channel: Uuid) -> Result<Vec<String>, CliError> {
    let raw = client
        .query(&json!({
            "kinds": [39002],
            "#d": [channel.to_string()],
            "limit": 1,
        }))
        .await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|error| CliError::Other(format!("invalid member snapshot response: {error}")))?;
    events.first().map(member_pubkeys).ok_or_else(|| {
        CliError::Usage(format!("channel {channel} has no readable member snapshot"))
    })
}

fn profile_name(event: &serde_json::Value) -> Option<String> {
    let content = event.get("content")?.as_str()?;
    let profile: serde_json::Value = serde_json::from_str(content).ok()?;
    profile
        .get("display_name")
        .and_then(serde_json::Value::as_str)
        .or_else(|| profile.get("name").and_then(serde_json::Value::as_str))
        .map(str::to_string)
}

async fn resolve_assignee(
    client: &BuzzClient,
    channel: Uuid,
    value: &str,
) -> Result<String, CliError> {
    let members = channel_members(client, channel).await?;
    if let Ok(pubkey) = PublicKey::parse(value.trim_start_matches('@')) {
        let hex = pubkey.to_hex();
        if members.iter().any(|member| member == &hex) {
            return Ok(hex);
        }
        return Err(CliError::Usage(format!(
            "assignee {hex} is not a member of channel {channel}"
        )));
    }

    let requested = value.trim_start_matches('@');
    if requested.is_empty() {
        return Err(CliError::Usage("--assignee must not be empty".into()));
    }
    let events = client
        .query_all(json!({ "kinds": [0], "authors": members }))
        .await?;
    let mut current_profiles = BTreeMap::new();
    for event in events {
        let Some(pubkey) = event.get("pubkey").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let order = (
            event
                .get("created_at")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            event
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
        );
        let replace = current_profiles
            .get(pubkey)
            .is_none_or(|(prior_order, _)| order > *prior_order);
        if replace {
            current_profiles.insert(pubkey.to_string(), (order, event));
        }
    }
    let mut matches: Vec<String> = current_profiles
        .into_iter()
        .filter(|(_, (_, event))| {
            profile_name(event).is_some_and(|name| name.eq_ignore_ascii_case(requested))
        })
        .map(|(pubkey, _)| pubkey)
        .collect();
    matches.sort();
    matches.dedup();
    match matches.as_slice() {
        [pubkey] => Ok(pubkey.clone()),
        [] => Err(CliError::Usage(format!(
            "no channel member has exact profile name {requested:?}"
        ))),
        _ => Err(CliError::Usage(format!(
            "assignee name {requested:?} is ambiguous; pass a pubkey"
        ))),
    }
}

fn print_write_with_task(resp: &str, task_id: Uuid, status: TaskStatus) {
    let normalized = normalize_write_response(resp);
    let mut value = serde_json::from_str::<serde_json::Value>(&normalized)
        .unwrap_or_else(|_| json!({ "response": resp }));
    value["task_id"] = json!(task_id);
    value["status"] = json!(status);
    println!("{value}");
}

#[allow(clippy::too_many_arguments)]
async fn assign(
    client: &BuzzClient,
    channel: String,
    assignee: String,
    title: String,
    deliverables: Vec<String>,
    verification: String,
    tracker_issue_id: Option<String>,
    dispatch_request_id: Option<String>,
    content: String,
) -> Result<(), CliError> {
    let channel = parse_uuid(&channel)?;
    let assignee = resolve_assignee(client, channel, &assignee).await?;
    let content = read_or_stdin(&content)?;
    let assigner = client.keys().public_key().to_hex();
    let envelope = TaskEnvelope {
        v: TASK_ENVELOPE_VERSION,
        task_id: Uuid::new_v4(),
        status: TaskStatus::Assigned,
        assignee: assignee.clone(),
        assigner,
        title,
        deliverables: parse_deliverables(&deliverables)?,
        verification: TaskVerification {
            method: verification,
            evidence_ref: None,
        },
        blocked_on: None,
        notes: None,
        tracker_issue_id: tracker_issue_id
            .as_deref()
            .map(|value| {
                Uuid::parse_str(value)
                    .map_err(|_| CliError::Usage(format!("invalid Tracker issue UUID: {value}")))
            })
            .transpose()?,
        dispatch_request_id: dispatch_request_id
            .as_deref()
            .map(|value| {
                Uuid::parse_str(value)
                    .map_err(|_| CliError::Usage(format!("invalid dispatch request UUID: {value}")))
            })
            .transpose()?,
    }
    .validate()
    .map_err(|error| sdk_error("invalid task envelope", error))?;
    let builder = buzz_sdk::task_envelope::build_task_message(
        channel,
        &content,
        &envelope,
        None,
        &[assignee.as_str()],
    )
    .map_err(|error| sdk_error("build task assignment", error))?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    print_write_with_task(&resp, envelope.task_id, envelope.status);
    Ok(())
}

struct TaskUpdateInput {
    task: String,
    status: String,
    channel: String,
    deliverables: Vec<String>,
    evidence: Option<String>,
    note: Option<String>,
    content: Option<String>,
}

async fn update(client: &BuzzClient, input: TaskUpdateInput) -> Result<(), CliError> {
    let TaskUpdateInput {
        task,
        status,
        channel,
        deliverables,
        evidence,
        note,
        content,
    } = input;
    let task_id = Uuid::parse_str(&task)
        .map_err(|_| CliError::Usage(format!("invalid task UUID: {task}")))?;
    let channel = parse_uuid(&channel)?;
    let status = parse_status(&status, false)?;
    let history: Vec<_> = query_task_events(client, Some(channel))
        .await?
        .into_iter()
        .filter(|event| event.envelope.task_id == task_id)
        .collect();
    let mut envelope = fold_task_events(history)
        .remove(&task_id)
        .ok_or_else(|| CliError::Usage(format!("task {task_id} not found in channel {channel}")))?
        .envelope;
    let author = client.keys().public_key().to_hex();
    if status == TaskStatus::Verified && author == envelope.assignee {
        return Err(CliError::Usage(
            "verified is reviewer-only; the assignee cannot self-verify".into(),
        ));
    }
    envelope.status = status;
    let additions = parse_deliverables(&deliverables)?;
    let mut seen: HashSet<(TaskDeliverableType, String)> = envelope
        .deliverables
        .iter()
        .map(|item| (item.kind, item.reference.clone()))
        .collect();
    for item in additions {
        if seen.insert((item.kind, item.reference.clone())) {
            envelope.deliverables.push(item);
        }
    }
    if let Some(evidence) = evidence {
        envelope.verification.evidence_ref = Some(evidence);
    }
    envelope.notes = note.clone();
    envelope.blocked_on = if status == TaskStatus::Blocked {
        note.clone()
    } else {
        None
    };
    envelope = envelope
        .validate()
        .map_err(|error| sdk_error("invalid task envelope", error))?;
    let human = match content {
        Some(content) => read_or_stdin(&content)?,
        None => format!("Task {}: {}.", envelope.title, envelope.status),
    };
    let builder =
        buzz_sdk::task_envelope::build_task_message(channel, &human, &envelope, None, &[])
            .map_err(|error| sdk_error("build task update", error))?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    print_write_with_task(&resp, task_id, status);
    Ok(())
}

async fn list(
    client: &BuzzClient,
    channel: Option<String>,
    status: Option<String>,
    assignee: Option<String>,
) -> Result<(), CliError> {
    let channel = channel.map(|value| parse_uuid(&value)).transpose()?;
    let status = status
        .as_deref()
        .map(|value| parse_status(value, true))
        .transpose()?;
    let assignee = match (assignee, channel) {
        (Some(value), Some(channel)) => Some(resolve_assignee(client, channel, &value).await?),
        (Some(value), None) => Some(
            PublicKey::parse(value.trim_start_matches('@'))
                .map_err(|_| CliError::Usage("name-based --assignee requires --channel".into()))?
                .to_hex(),
        ),
        (None, _) => None,
    };
    let current = fold_task_events(query_task_events(client, channel).await?);
    let rows: Vec<_> = current
        .into_values()
        .filter(|event| status.is_none_or(|status| event.envelope.status == status))
        .filter(|event| {
            assignee
                .as_ref()
                .is_none_or(|assignee| event.envelope.assignee == *assignee)
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into())
    );
    Ok(())
}

async fn get(client: &BuzzClient, task: String) -> Result<(), CliError> {
    let task_id = Uuid::parse_str(&task)
        .map_err(|_| CliError::Usage(format!("invalid task UUID: {task}")))?;
    let mut history: Vec<_> = query_task_events(client, None)
        .await?
        .into_iter()
        .filter(|event| event.envelope.task_id == task_id)
        .collect();
    history.sort_by(|left, right| {
        (left.created_at, left.event_id.as_str()).cmp(&(right.created_at, right.event_id.as_str()))
    });
    if history.is_empty() {
        return Err(CliError::Usage(format!("task {task_id} not found")));
    }
    println!(
        "{}",
        serde_json::to_string(&history).unwrap_or_else(|_| "[]".into())
    );
    Ok(())
}

pub async fn dispatch(cmd: TasksCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        TasksCmd::Assign {
            channel,
            assignee,
            title,
            deliverables,
            verification,
            tracker_issue_id,
            dispatch_request_id,
            content,
        } => {
            assign(
                client,
                channel,
                assignee,
                title,
                deliverables,
                verification,
                tracker_issue_id,
                dispatch_request_id,
                content,
            )
            .await
        }
        TasksCmd::Update {
            task,
            status,
            channel,
            deliverables,
            evidence,
            note,
            content,
        } => {
            update(
                client,
                TaskUpdateInput {
                    task,
                    status,
                    channel,
                    deliverables,
                    evidence,
                    note,
                    content,
                },
            )
            .await
        }
        TasksCmd::List {
            channel,
            status,
            assignee,
        } => list(client, channel, status, assignee).await,
        TasksCmd::Get { task } => get(client, task).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deliverable_parser_accepts_contract_types() {
        for kind in ["pr", "commit", "file", "doc", "event", "url"] {
            assert_eq!(
                parse_deliverable(&format!("{kind}=reference"))
                    .unwrap()
                    .reference,
                "reference"
            );
        }
    }

    #[test]
    fn update_status_rejects_assigned_and_unknown() {
        assert!(parse_status("assigned", false).is_err());
        assert!(parse_status("complete", false).is_err());
        assert_eq!(parse_status("done", false).unwrap(), TaskStatus::Done);
    }
}
