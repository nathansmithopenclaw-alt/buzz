//! `buzz track` — agent-first workspace Tracker CLI.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use base64::Engine;
use buzz_core::kind::{
    KIND_NIP29_GROUP_MEMBERS, KIND_PROJECT, KIND_TRACKER_COMMENT, KIND_TRACKER_ISSUE,
    KIND_TRACKER_PROJECT_ACTIVITY, KIND_TRACKER_REVISION,
};
use buzz_sdk::task_envelope::{
    fold_task_events, TaskEnvelope, TaskEnvelopeEvent, TaskStatus, TaskVerification,
    TASK_ENVELOPE_VERSION,
};
use buzz_sdk::tracker::{
    build_tracker_comment, build_tracker_issue, build_tracker_project_activity,
    build_tracker_revision, canonicalize_relation, default_project_statuses, fold_tracker_events,
    fold_tracker_projects, parse_tracker_event, parse_tracker_project, Due, DuePrecision,
    ExternalRef, ImportProvenance, IssueComment, IssueProjection, IssueRevision, IssueRoot,
    IssueStatus, Priority, ProjectActivity, ProjectActivityOperation, RelationDirection,
    RevisionSet, SetOperations, TrackerAuthorization, TrackerEvent, TrackerPayload, TrackerProject,
    TrackerProjectSnapshot, TrackerRole, TRACKER_VERSION,
};
use buzz_sdk::{build_delete_addressable, build_project_with_tags};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use nostr::{Event, PublicKey, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::client::{normalize_write_response, BuzzClient};
use crate::commands::tasks::{parse_deliverables, query_task_events, resolve_assignee};
use crate::error::CliError;
use crate::validate::read_or_stdin;
use crate::{OutputFormat, TrackCmd, TrackFilterArgs, TrackMutationArgs, TrackProjectCmd};

const TRACKER_CACHE_ENV: &str = "BUZZ_TRACKER_CACHE_DIR";
const DEFAULT_TIMEZONE: &str = "America/New_York";
const PROJECT_UPDATE_INTENT_TAG: &str = "buzz-project-update";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ProjectStatusMigration {
    issue_id: Uuid,
    root_event: String,
    parent_event: String,
    revision: u64,
    from: IssueStatus,
    to: IssueStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ProjectUpdateIntent {
    v: u8,
    request_id: Uuid,
    before: TrackerProjectSnapshot,
    reason: String,
    status_maps: Vec<(IssueStatus, IssueStatus)>,
    migrations: Vec<ProjectStatusMigration>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct EventCursor {
    created_at: u64,
    event_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CachedProjection {
    workspace_id: Option<Uuid>,
    cursor: Option<EventCursor>,
    events: Vec<Event>,
}

struct TrackerContext {
    workspace_id: Uuid,
    cache_path: PathBuf,
    cache: CachedProjection,
    parsed: Vec<TrackerEvent>,
    fold: buzz_sdk::tracker::TrackerFold,
    projects: buzz_sdk::tracker::TrackerProjectProjection,
    warnings: Vec<String>,
}

impl TrackerContext {
    fn issue(&self, value: &str) -> Result<&IssueProjection, CliError> {
        let parsed_uuid = Uuid::parse_str(value).ok();
        let mut matches = self.fold.issues.values().filter(|issue| {
            parsed_uuid == Some(issue.issue_id) || issue.identifier.eq_ignore_ascii_case(value)
        });
        let found = matches
            .next()
            .ok_or_else(|| CliError::NotFound(format!("Tracker issue {value:?} not found")))?;
        if matches.next().is_some() {
            return Err(CliError::Conflict(format!(
                "Tracker identifier {value:?} is ambiguous; pass the issue UUID"
            )));
        }
        Ok(found)
    }

    fn project(&self, value: &str) -> Result<&TrackerProject, CliError> {
        let parsed_uuid = Uuid::parse_str(value).ok();
        let mut matches = self.projects.projects.values().filter(|project| {
            parsed_uuid == Some(project.project_id) || project.slug.eq_ignore_ascii_case(value)
        });
        let found = matches
            .next()
            .ok_or_else(|| CliError::NotFound(format!("Tracker project {value:?} not found")))?;
        if matches.next().is_some() {
            return Err(CliError::Conflict(format!(
                "Tracker project key {value:?} is ambiguous; pass the project UUID"
            )));
        }
        Ok(found)
    }

    fn event_time(&self, event_id: &str) -> u64 {
        self.cache
            .events
            .iter()
            .find(|event| event.id.to_hex() == event_id)
            .map(|event| event.created_at.as_secs())
            .unwrap_or(0)
    }

    fn revision_number(&self, issue: &IssueProjection) -> u64 {
        self.parsed
            .iter()
            .find(|event| event.event_id == issue.tip_event)
            .and_then(|event| match &event.payload {
                TrackerPayload::Revision(revision) => Some(revision.revision),
                TrackerPayload::Root(_) => Some(0),
                TrackerPayload::Comment(_) => None,
            })
            .unwrap_or(0)
    }

    fn add_event(&mut self, event: Event, authorization: &TrackerAuthorization) {
        merge_events(&mut self.cache.events, [event]);
        self.cache.cursor = max_cursor(&self.cache.events);
        let (parsed, mut warnings) = parse_cached_events(&self.cache.events);
        self.parsed = parsed;
        self.fold = fold_tracker_events(self.parsed.clone(), authorization);
        self.warnings.append(&mut warnings);
    }

    fn persist(&self) -> Result<(), CliError> {
        write_cache(&self.cache_path, &self.cache)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ListCursor {
    sort: String,
    primary: String,
    updated_at: u64,
    issue_id: Uuid,
}

fn usage(context: &str, error: impl std::fmt::Display) -> CliError {
    CliError::Usage(format!("{context}: {error}"))
}

fn parse_workspace(value: Option<String>) -> Result<Uuid, CliError> {
    let value = value.ok_or_else(|| {
        CliError::Usage("--workspace is required unless BUZZ_WORKSPACE_ID is configured".into())
    })?;
    Uuid::parse_str(&value).map_err(|_| CliError::Usage(format!("invalid workspace UUID: {value}")))
}

fn parse_request_id(value: Option<&str>) -> Result<Uuid, CliError> {
    value
        .map(|value| {
            Uuid::parse_str(value)
                .map_err(|_| CliError::Usage(format!("invalid request UUID: {value}")))
        })
        .transpose()
        .map(|value| value.unwrap_or_else(Uuid::new_v4))
}

fn cache_path(workspace_id: Uuid) -> Result<PathBuf, CliError> {
    let base = std::env::var_os(TRACKER_CACHE_ENV)
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|path| path.join("buzz").join("tracker")))
        .ok_or_else(|| CliError::Other("cannot determine Tracker cache directory".into()))?;
    Ok(base.join(format!("{workspace_id}.json")))
}

fn read_cache(path: &Path, workspace_id: Uuid) -> Result<CachedProjection, CliError> {
    let raw = match fs::read(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CachedProjection {
                workspace_id: Some(workspace_id),
                ..CachedProjection::default()
            });
        }
        Err(error) => {
            return Err(CliError::Other(format!(
                "Tracker cache read failed: {error}"
            )))
        }
    };
    let cache: CachedProjection = serde_json::from_slice(&raw)
        .map_err(|error| CliError::Other(format!("Tracker cache is malformed: {error}")))?;
    if cache.workspace_id != Some(workspace_id) {
        return Err(CliError::Other(
            "Tracker cache workspace identity mismatch".into(),
        ));
    }
    Ok(cache)
}

fn write_cache(path: &Path, cache: &CachedProjection) -> Result<(), CliError> {
    let parent = path
        .parent()
        .ok_or_else(|| CliError::Other("Tracker cache path has no parent".into()))?;
    fs::create_dir_all(parent)
        .map_err(|error| CliError::Other(format!("Tracker cache directory failed: {error}")))?;
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("tracker"),
        std::process::id()
    ));
    let raw = serde_json::to_vec(cache)
        .map_err(|error| CliError::Other(format!("Tracker cache serialization failed: {error}")))?;
    let mut file = fs::File::create(&temp)
        .map_err(|error| CliError::Other(format!("Tracker cache temp create failed: {error}")))?;
    file.write_all(&raw)
        .and_then(|_| file.sync_all())
        .map_err(|error| CliError::Other(format!("Tracker cache temp write failed: {error}")))?;
    fs::rename(&temp, path)
        .map_err(|error| CliError::Other(format!("Tracker cache replace failed: {error}")))
}

fn merge_events(events: &mut Vec<Event>, incoming: impl IntoIterator<Item = Event>) {
    let mut by_id: BTreeMap<String, Event> = events
        .drain(..)
        .map(|event| (event.id.to_hex(), event))
        .collect();
    for event in incoming {
        by_id.insert(event.id.to_hex(), event);
    }
    *events = by_id.into_values().collect();
    events.sort_by(|left, right| {
        (left.created_at.as_secs(), left.id.to_hex())
            .cmp(&(right.created_at.as_secs(), right.id.to_hex()))
    });
}

fn max_cursor(events: &[Event]) -> Option<EventCursor> {
    events
        .iter()
        .map(|event| EventCursor {
            created_at: event.created_at.as_secs(),
            event_id: event.id.to_hex(),
        })
        .max_by(|left, right| {
            (left.created_at, left.event_id.as_str())
                .cmp(&(right.created_at, right.event_id.as_str()))
        })
}

fn parse_cached_events(events: &[Event]) -> (Vec<TrackerEvent>, Vec<String>) {
    let now = Utc::now().timestamp().max(0) as u64;
    let mut parsed = Vec::new();
    let mut warnings = Vec::new();
    for event in events {
        if !matches!(
            event.kind.as_u16() as u32,
            KIND_TRACKER_ISSUE | KIND_TRACKER_REVISION | KIND_TRACKER_COMMENT
        ) {
            continue;
        }
        match parse_tracker_event(event, now) {
            Ok(event) => parsed.push(event),
            Err(error) => warnings.push(format!(
                "ignored malformed Tracker event {}: {error}",
                event.id.to_hex()
            )),
        }
    }
    (parsed, warnings)
}

async fn tracker_authorization(
    client: &BuzzClient,
    workspace_id: Uuid,
) -> Result<TrackerAuthorization, CliError> {
    let events = client
        .query_paginated(
            json!({
                "kinds": [KIND_NIP29_GROUP_MEMBERS],
                "#d": [workspace_id.to_string()],
            }),
            1,
        )
        .await?;
    let event = events.first().ok_or_else(|| {
        CliError::Auth(format!(
            "workspace {workspace_id} has no readable membership snapshot"
        ))
    })?;
    let mut roles = Vec::new();
    if let Some(author) = event.get("pubkey").and_then(Value::as_str) {
        roles.push((author.to_string(), TrackerRole::Maintainer));
    }
    for tag in event
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .filter(|tag| tag.first().and_then(Value::as_str) == Some("p"))
    {
        let Some(pubkey) = tag.get(1).and_then(Value::as_str) else {
            continue;
        };
        if PublicKey::parse(pubkey).is_err() {
            continue;
        }
        let role = match tag.get(3).and_then(Value::as_str).unwrap_or("member") {
            "owner" | "admin" | "bot" => TrackerRole::Maintainer,
            _ => TrackerRole::Member,
        };
        roles.push((pubkey.to_string(), role));
    }
    if let Some(owner) = client.auth_tag_owner_hex() {
        roles.push((owner, TrackerRole::Maintainer));
    }
    Ok(TrackerAuthorization::new(roles))
}

async fn load_context(
    client: &BuzzClient,
    workspace_id: Uuid,
) -> Result<(TrackerContext, TrackerAuthorization), CliError> {
    let path = cache_path(workspace_id)?;
    let mut cache = read_cache(&path, workspace_id)?;
    let mut filter = json!({
        "kinds": [
            KIND_TRACKER_ISSUE,
            KIND_TRACKER_REVISION,
            KIND_TRACKER_COMMENT,
            KIND_TRACKER_PROJECT_ACTIVITY,
        ],
        "#h": [workspace_id.to_string()],
    });
    if let Some(cursor) = &cache.cursor {
        // Inclusive same-second replay plus ID dedupe prevents cursor gaps.
        filter["since"] = json!(cursor.created_at);
    }
    let incoming: Vec<Event> = client
        .query_all(filter)
        .await?
        .into_iter()
        .map(|value| {
            serde_json::from_value(value)
                .map_err(|error| CliError::Other(format!("invalid Tracker event JSON: {error}")))
        })
        .collect::<Result<_, _>>()?;
    merge_events(&mut cache.events, incoming);
    cache.cursor = max_cursor(&cache.events);
    write_cache(&path, &cache)?;

    let authorization = tracker_authorization(client, workspace_id).await?;
    let (parsed, mut warnings) = parse_cached_events(&cache.events);
    let fold = fold_tracker_events(parsed.clone(), &authorization);
    warnings.extend(
        fold.diagnostics
            .iter()
            .map(|diagnostic| format!("{diagnostic:?}")),
    );

    let project_events: Vec<Event> = client
        .query_all(json!({
            "kinds": [KIND_PROJECT],
            "#buzz-workspace": [workspace_id.to_string()],
        }))
        .await?
        .into_iter()
        .map(|value| {
            serde_json::from_value(value)
                .map_err(|error| CliError::Other(format!("invalid project event JSON: {error}")))
        })
        .collect::<Result<_, _>>()?;
    let signer = client.keys().public_key().to_hex();
    let projects = fold_tracker_projects(project_events, workspace_id, &signer);
    warnings.extend(
        projects
            .diagnostics
            .iter()
            .map(|diagnostic| format!("{diagnostic:?}")),
    );
    Ok((
        TrackerContext {
            workspace_id,
            cache_path: path,
            cache,
            parsed,
            fold,
            projects,
            warnings,
        },
        authorization,
    ))
}

fn parse_status(value: &str) -> Result<IssueStatus, CliError> {
    IssueStatus::from_str(value).map_err(|error| usage("invalid status", error))
}

fn parse_priority(value: &str) -> Result<Priority, CliError> {
    Priority::from_str(value).map_err(|error| usage("invalid priority", error))
}

fn parse_estimate(value: &str) -> Result<u32, CliError> {
    if value.is_empty() || value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CliError::Usage(
            "--estimate requires a duration such as 90m, 2h, or 1d (bare points are not accepted)"
                .into(),
        ));
    }
    let (unit_offset, unit) = value
        .char_indices()
        .next_back()
        .ok_or_else(|| CliError::Usage(format!("invalid estimate duration: {value}")))?;
    let number = &value[..unit_offset];
    let number: u32 = number
        .parse()
        .map_err(|_| CliError::Usage(format!("invalid estimate duration: {value}")))?;
    let multiplier = match unit {
        'm' => 1,
        'h' => 60,
        'd' => 8 * 60,
        _ => {
            return Err(CliError::Usage(format!(
                "invalid estimate unit in {value:?}; use m, h, or d"
            )))
        }
    };
    number
        .checked_mul(multiplier)
        .filter(|minutes| *minutes > 0)
        .ok_or_else(|| CliError::Usage("estimate duration is out of range".into()))
}

fn parse_due(value: &str, timezone: &str) -> Result<Due, CliError> {
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| CliError::Usage(format!("invalid due date {value:?}; use YYYY-MM-DD")))?;
    let timezone = Tz::from_str(timezone)
        .map_err(|_| CliError::Usage(format!("invalid IANA timezone: {timezone}")))?;
    let local = timezone
        .from_local_datetime(
            &date
                .and_hms_opt(0, 0, 0)
                .ok_or_else(|| CliError::Usage("invalid due date".into()))?,
        )
        .single()
        .ok_or_else(|| CliError::Usage("due date does not map to one local midnight".into()))?;
    Ok(Due {
        value: local.timestamp() as u64,
        precision: DuePrecision::Date,
        timezone: timezone.name().to_string(),
    })
}

fn parse_external(value: &str) -> Result<ExternalRef, CliError> {
    let (kind, reference) = value.split_once(':').ok_or_else(|| {
        CliError::Usage(format!(
            "external reference must be system:id (got {value:?})"
        ))
    })?;
    if kind.trim().is_empty() || reference.trim().is_empty() {
        return Err(CliError::Usage(format!(
            "external reference must be system:id (got {value:?})"
        )));
    }
    Ok(ExternalRef {
        kind: kind.trim().to_string(),
        reference: reference.trim().to_string(),
    })
}

fn parse_import_json(value: Option<&str>) -> Result<Option<ImportProvenance>, CliError> {
    value
        .map(|value| {
            let raw = if let Some(path) = value.strip_prefix('@') {
                fs::read_to_string(path).map_err(|error| {
                    CliError::Usage(format!("cannot read import provenance {path:?}: {error}"))
                })?
            } else {
                value.to_string()
            };
            serde_json::from_str(&raw).map_err(|error| {
                CliError::Usage(format!("invalid import provenance JSON: {error}"))
            })
        })
        .transpose()
}

fn normalize_labels(values: impl IntoIterator<Item = String>) -> Result<Vec<String>, CliError> {
    let mut labels = BTreeSet::new();
    for value in values {
        let normalized = value.trim().to_lowercase();
        if normalized.is_empty() || normalized.len() > 64 {
            return Err(CliError::Usage(
                "labels must contain 1..=64 UTF-8 bytes".into(),
            ));
        }
        labels.insert(normalized);
    }
    if labels.len() > 16 {
        return Err(CliError::Usage("at most 16 labels are allowed".into()));
    }
    Ok(labels.into_iter().collect())
}

fn read_optional_stdin(value: Option<String>) -> Result<Option<String>, CliError> {
    value.map(|value| read_or_stdin(&value)).transpose()
}

fn due_json(due: Option<&Due>) -> Value {
    let Some(due) = due else {
        return Value::Null;
    };
    if due.precision == DuePrecision::Date {
        if let Ok(timezone) = Tz::from_str(&due.timezone) {
            if let Some(value) = timezone.timestamp_opt(due.value as i64, 0).single() {
                return json!(value.format("%Y-%m-%d").to_string());
            }
        }
    }
    json!(DateTime::<Utc>::from_timestamp(due.value as i64, 0)
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|| due.value.to_string()))
}

fn rfc3339(timestamp: u64) -> String {
    DateTime::<Utc>::from_timestamp(timestamp as i64, 0)
        .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| timestamp.to_string())
}

fn project_ref(context: &TrackerContext, project_id: Option<Uuid>) -> Value {
    let Some(project_id) = project_id else {
        return Value::Null;
    };
    match context.projects.projects.get(&project_id) {
        Some(project) => json!({"project_id": project_id, "slug": project.slug}),
        None => json!({"project_id": project_id, "slug": null}),
    }
}

fn issue_summary(
    context: &TrackerContext,
    issue: &IssueProjection,
    active_task_id: Option<Uuid>,
) -> Value {
    json!({
        "issue_id": issue.issue_id,
        "identifier": issue.identifier,
        "title": issue.title,
        "status": issue.status,
        "priority": issue.priority,
        "assignee_pubkey": issue.assignee,
        "project": project_ref(context, issue.project_id),
        "labels": issue.labels,
        "due": due_json(issue.due.as_ref()),
        "estimate": issue.estimate_minutes,
        "home_channel_id": issue.home_channel_id,
        "active_task_id": active_task_id,
        "created_at": rfc3339(issue.root_created_at),
        "updated_at": rfc3339(context.event_time(&issue.tip_event)),
        "revision": context.revision_number(issue),
        "revision_event_id": issue.tip_event,
        "root_event_id": issue.root_event,
    })
}

fn compact_issue(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.remove("body");
        object.remove("relations");
        object.remove("external_refs");
        object.remove("activity");
        object.remove("comments");
        object.remove("linked_tasks");
    }
    value
}

fn semantic_event_tags(event: &Event) -> Vec<Vec<String>> {
    let mut tags: Vec<_> = event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .filter(|tag| {
            let name = tag.first().map(String::as_str);
            name != Some("auth")
                && !(event.kind.as_u16() as u32 == KIND_TRACKER_REVISION && name == Some("e"))
        })
        .collect();
    tags.sort();
    tags
}

fn semantic_event_content(event: &Event) -> Value {
    let Ok(mut value) = serde_json::from_str::<Value>(&event.content) else {
        return Value::String(event.content.clone());
    };
    if event.kind.as_u16() as u32 == KIND_TRACKER_REVISION {
        if let Some(object) = value.as_object_mut() {
            object.remove("parent_event");
            object.remove("revision");
        }
    }
    value
}

fn same_semantic_request(existing: &Event, candidate: &Event) -> bool {
    existing.kind == candidate.kind
        && semantic_event_content(existing) == semantic_event_content(candidate)
        && semantic_event_tags(existing) == semantic_event_tags(candidate)
}

async fn existing_request_event(
    client: &BuzzClient,
    workspace_id: Uuid,
    request_id: Uuid,
) -> Result<Option<Event>, CliError> {
    let events = client
        .query_paginated(
            json!({
                "kinds": [
                    KIND_TRACKER_ISSUE,
                    KIND_TRACKER_REVISION,
                    KIND_TRACKER_COMMENT,
                    KIND_TRACKER_PROJECT_ACTIVITY,
                    KIND_PROJECT,
                ],
                "#h": [workspace_id.to_string()],
                "#request": [request_id.to_string()],
            }),
            2,
        )
        .await?;
    let parsed: Vec<Event> = events
        .into_iter()
        .map(|value| {
            serde_json::from_value(value)
                .map_err(|error| CliError::Other(format!("invalid request event JSON: {error}")))
        })
        .collect::<Result<_, _>>()?;
    if parsed.len() > 1 {
        return Err(CliError::Conflict(format!(
            "request {request_id} already names multiple events"
        )));
    }
    Ok(parsed.into_iter().next())
}

async fn submit_idempotent(
    client: &BuzzClient,
    workspace_id: Uuid,
    request_id: Uuid,
    event: Event,
) -> Result<(String, bool), CliError> {
    if let Some(existing) = existing_request_event(client, workspace_id, request_id).await? {
        if same_semantic_request(&existing, &event) {
            return Ok((existing.id.to_hex(), true));
        }
        return Err(CliError::Conflict(format!(
            "request {request_id} was already used with different semantic input"
        )));
    }
    let raw = client.submit_event(event.clone()).await?;
    let response: Value = serde_json::from_str(&normalize_write_response(&raw))
        .map_err(|error| CliError::Other(format!("invalid relay write receipt: {error}")))?;
    if response.get("accepted").and_then(Value::as_bool) != Some(true) {
        return Err(CliError::Other(format!(
            "relay rejected Tracker event: {}",
            response
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown reason")
        )));
    }
    Ok((event.id.to_hex(), false))
}

fn parse_timestamp(value: &str) -> Result<u64, CliError> {
    if let Ok(value) = value.parse::<u64>() {
        return Ok(value);
    }
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.timestamp().max(0) as u64)
        .map_err(|_| CliError::Usage(format!("invalid timestamp {value:?}")))
}

fn encode_cursor<T: Serialize>(value: &T) -> Result<String, CliError> {
    let raw = serde_json::to_vec(value)
        .map_err(|error| CliError::Other(format!("cursor serialization failed: {error}")))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw))
}

fn decode_cursor<T: for<'de> Deserialize<'de>>(value: &str) -> Result<T, CliError> {
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| CliError::Usage("invalid opaque cursor".into()))?;
    serde_json::from_slice(&raw).map_err(|_| CliError::Usage("invalid opaque cursor".into()))
}

fn validate_candidate(
    context: &TrackerContext,
    authorization: &TrackerAuthorization,
    event: &Event,
    issue_id: Uuid,
) -> Result<(), CliError> {
    let now = Utc::now().timestamp().max(0) as u64;
    let parsed =
        parse_tracker_event(event, now).map_err(|error| usage("invalid Tracker write", error))?;
    let mut events = context.parsed.clone();
    events.push(parsed);
    let folded = fold_tracker_events(events, authorization);
    let accepted = folded
        .issues
        .get(&(context.workspace_id, issue_id))
        .is_some_and(|issue| issue.tip_event == event.id.to_hex());
    if !accepted {
        let reasons: Vec<_> = folded
            .diagnostics
            .iter()
            .filter_map(|diagnostic| match diagnostic {
                buzz_sdk::tracker::FoldDiagnostic::InvalidEvent { event_id, reason }
                    if event_id == &event.id.to_hex() =>
                {
                    Some(reason.clone())
                }
                _ => None,
            })
            .collect();
        return Err(CliError::Conflict(if reasons.is_empty() {
            "Tracker write would not become the canonical revision tip".into()
        } else {
            reasons.join("; ")
        }));
    }
    Ok(())
}

async fn workspace_prefix(client: &BuzzClient, workspace_id: Uuid) -> Result<String, CliError> {
    if let Ok(value) = std::env::var("BUZZ_TRACKER_PREFIX") {
        let value = value.trim().trim_end_matches('-').to_ascii_uppercase();
        if !value.is_empty()
            && value.len() <= 8
            && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Ok(value);
        }
        return Err(CliError::Usage(
            "BUZZ_TRACKER_PREFIX must be 1..=8 ASCII alphanumeric characters".into(),
        ));
    }
    let events = client
        .query_paginated(
            json!({"kinds": [39000], "#d": [workspace_id.to_string()]}),
            1,
        )
        .await?;
    let name = events
        .first()
        .and_then(|event| event.get("tags"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .find(|tag| tag.first().and_then(Value::as_str) == Some("name"))
        .and_then(|tag| tag.get(1).and_then(Value::as_str));
    let mut prefix = String::new();
    if let Some(name) = name {
        for word in name.split(|character: char| !character.is_ascii_alphanumeric()) {
            if let Some(character) = word.chars().next() {
                prefix.push(character.to_ascii_uppercase());
            }
            if prefix.len() == 4 {
                break;
            }
        }
    }
    if prefix.is_empty() {
        prefix = "TRK".into();
    }
    Ok(prefix)
}

fn allocate_identifier(prefix: &str, request_id: Uuid) -> String {
    format!("{prefix}-{}", request_id.simple())
}

async fn resolve_issue_assignee(
    client: &BuzzClient,
    channel_id: Uuid,
    value: &str,
) -> Result<String, CliError> {
    if value == "me" {
        return Ok(client.keys().public_key().to_hex());
    }
    resolve_assignee(client, channel_id, value).await
}

#[allow(clippy::too_many_arguments)]
async fn cmd_create(
    client: &BuzzClient,
    workspace: Option<String>,
    requested_issue_id: Option<String>,
    requested_identifier: Option<String>,
    channel: String,
    title: String,
    body: String,
    status: String,
    priority: Option<String>,
    assignee: Option<String>,
    project: Option<String>,
    labels: Vec<String>,
    due: Option<String>,
    timezone: String,
    estimate: Option<String>,
    origin_message: Option<String>,
    external_ids: Vec<String>,
    mutation: TrackMutationArgs,
    format: &OutputFormat,
) -> Result<(), CliError> {
    let workspace_id = parse_workspace(workspace)?;
    let channel_id = Uuid::parse_str(&channel)
        .map_err(|_| CliError::Usage(format!("invalid channel UUID: {channel}")))?;
    let (mut context, authorization) = load_context(client, workspace_id).await?;
    let request_id = parse_request_id(mutation.request_id.as_deref())?;
    let existing = existing_request_event(client, workspace_id, request_id).await?;
    let existing_root = existing
        .as_ref()
        .map(|event| {
            let parsed = parse_tracker_event(event, Utc::now().timestamp().max(0) as u64)
                .map_err(|error| usage("parse existing create request", error))?;
            match parsed.payload {
                TrackerPayload::Root(root) => Ok(root),
                _ => Err(CliError::Conflict(format!(
                    "request {request_id} was already used by another Tracker mutation"
                ))),
            }
        })
        .transpose()?;
    let issue_id = requested_issue_id
        .as_deref()
        .map(|value| {
            Uuid::parse_str(value)
                .map_err(|_| CliError::Usage(format!("invalid issue UUID: {value}")))
        })
        .transpose()?
        .or_else(|| existing_root.as_ref().map(|root| root.issue_id))
        .unwrap_or(request_id);
    let status = parse_status(&status)?;
    let priority = priority.as_deref().map(parse_priority).transpose()?;
    let assignee = match assignee {
        Some(value) => Some(resolve_issue_assignee(client, channel_id, &value).await?),
        None => None,
    };
    let project_id = project
        .as_deref()
        .map(|value| context.project(value).map(|project| project.project_id))
        .transpose()?;
    let due = due
        .as_deref()
        .map(|value| parse_due(value, &timezone))
        .transpose()?;
    let estimate_minutes = estimate.as_deref().map(parse_estimate).transpose()?;
    let body = read_or_stdin(&body)?;
    let labels = normalize_labels(labels)?;
    let external_refs = external_ids
        .iter()
        .map(|value| parse_external(value))
        .collect::<Result<Vec<_>, _>>()?;
    let prefix = workspace_prefix(client, workspace_id).await?;
    let root = IssueRoot {
        v: TRACKER_VERSION,
        op: "create".into(),
        issue_id,
        identifier: requested_identifier
            .or_else(|| existing_root.as_ref().map(|root| root.identifier.clone()))
            .unwrap_or_else(|| allocate_identifier(&prefix, request_id)),
        title,
        body,
        status,
        priority,
        assignee,
        project_id,
        due,
        estimate_minutes,
        home_channel_id: Some(channel_id),
        source_event_id: origin_message,
        workflow: "default-v1".into(),
        labels,
        external_refs,
        request_id: Some(request_id),
        import: parse_import_json(mutation.import_json.as_deref())?,
    };
    let event = client.sign_event(
        build_tracker_issue(workspace_id, &root)
            .map_err(|error| usage("build Tracker issue", error))?,
    )?;
    if existing.is_some() {
        let (event_id, replayed) =
            submit_idempotent(client, workspace_id, request_id, event).await?;
        let issue = context.issue(&issue_id.to_string())?;
        let mut summary = issue_summary(&context, issue, None);
        if matches!(format, OutputFormat::Compact) {
            summary = compact_issue(summary);
        }
        println!(
            "{}",
            json!({
                "accepted": true,
                "operation": "track.create",
                "request_id": request_id,
                "event_ids": [event_id],
                "replayed": replayed,
                "issue": summary,
                "warnings": context.warnings,
            })
        );
        return Ok(());
    }
    validate_candidate(&context, &authorization, &event, issue_id)?;
    let (event_id, replayed) =
        submit_idempotent(client, workspace_id, request_id, event.clone()).await?;
    if !replayed {
        context.add_event(event, &authorization);
        context.persist()?;
    }
    let issue = context.issue(&issue_id.to_string())?;
    let mut summary = issue_summary(&context, issue, None);
    if matches!(format, OutputFormat::Compact) {
        summary = compact_issue(summary);
    }
    println!(
        "{}",
        json!({
            "accepted": true,
            "operation": "track.create",
            "request_id": request_id,
            "event_ids": [event_id],
            "replayed": replayed,
            "issue": summary,
            "warnings": context.warnings,
        })
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_update(
    client: &BuzzClient,
    workspace: Option<String>,
    issue_key: String,
    title: Option<String>,
    body: Option<String>,
    status: Option<String>,
    priority: Option<String>,
    assignee: Option<String>,
    project: Option<String>,
    due: Option<String>,
    timezone: String,
    estimate: Option<String>,
    add_label: Vec<String>,
    remove_label: Vec<String>,
    channel: Option<String>,
    mutation: TrackMutationArgs,
    format: &OutputFormat,
) -> Result<(), CliError> {
    let workspace_id = parse_workspace(workspace)?;
    let (mut context, authorization) = load_context(client, workspace_id).await?;
    let current = context.issue(&issue_key)?.clone();
    if mutation
        .if_revision
        .as_ref()
        .is_some_and(|expected| expected != &current.tip_event)
    {
        return Err(CliError::Conflict(format!(
            "current revision is {}; expected {}",
            current.tip_event,
            mutation.if_revision.as_deref().unwrap_or_default()
        )));
    }
    let request_id = parse_request_id(mutation.request_id.as_deref())?;
    let mut set = RevisionSet {
        title,
        body: read_optional_stdin(body)?,
        status: status.as_deref().map(parse_status).transpose()?,
        ..RevisionSet::default()
    };
    let mut clear = Vec::new();
    if let Some(value) = priority {
        if value == "none" {
            clear.push("priority".into());
        } else {
            set.priority = Some(parse_priority(&value)?);
        }
    }
    if let Some(value) = assignee {
        if value == "none" {
            clear.push("assignee".into());
        } else {
            let channel_id = current.home_channel_id.ok_or_else(|| {
                CliError::Usage("name-based assignment requires an issue home channel".into())
            })?;
            set.assignee = Some(resolve_issue_assignee(client, channel_id, &value).await?);
        }
    }
    if let Some(value) = project {
        if value == "none" {
            clear.push("project_id".into());
        } else {
            set.project_id = Some(context.project(&value)?.project_id);
        }
    }
    if let Some(value) = due {
        if value == "none" {
            clear.push("due".into());
        } else {
            set.due = Some(parse_due(&value, &timezone)?);
        }
    }
    if let Some(value) = estimate {
        if value == "none" {
            clear.push("estimate_minutes".into());
        } else {
            set.estimate_minutes = Some(parse_estimate(&value)?);
        }
    }
    if let Some(value) = channel {
        set.home_channel_id = Some(
            Uuid::parse_str(&value)
                .map_err(|_| CliError::Usage(format!("invalid channel UUID: {value}")))?,
        );
    }
    if !add_label.is_empty() || !remove_label.is_empty() {
        let mut labels = current.labels.clone();
        for value in normalize_labels(add_label)? {
            labels.insert(value);
        }
        for value in normalize_labels(remove_label)? {
            labels.remove(&value);
        }
        set.labels = Some(labels.into_iter().collect());
    }
    let reason = read_optional_stdin(mutation.reason)?;
    let revision = IssueRevision {
        v: TRACKER_VERSION,
        op: "revise".into(),
        issue_id: current.issue_id,
        root_event: current.root_event.clone(),
        parent_event: current.tip_event.clone(),
        revision: context.revision_number(&current) + 1,
        set,
        clear,
        relations: SetOperations::default(),
        external_refs: SetOperations::default(),
        reason,
        request_id: Some(request_id),
        import: parse_import_json(mutation.import_json.as_deref())?,
    };
    let event = client.sign_event(
        build_tracker_revision(workspace_id, &revision)
            .map_err(|error| usage("build Tracker revision", error))?,
    )?;
    validate_candidate(&context, &authorization, &event, current.issue_id)?;
    let (event_id, replayed) =
        submit_idempotent(client, workspace_id, request_id, event.clone()).await?;
    if !replayed {
        context.add_event(event, &authorization);
        context.persist()?;
    }
    let issue = context.issue(&current.issue_id.to_string())?;
    let mut warnings = context.warnings.clone();
    if revision.set.assignee.is_some() {
        warnings.push("no task envelope created — use `buzz track dispatch` to start work".into());
    }
    let mut summary = issue_summary(&context, issue, None);
    if matches!(format, OutputFormat::Compact) {
        summary = compact_issue(summary);
    }
    println!(
        "{}",
        json!({
            "accepted": true,
            "operation": "track.update",
            "request_id": request_id,
            "event_ids": [event_id],
            "replayed": replayed,
            "issue": summary,
            "warnings": warnings,
        })
    );
    Ok(())
}

fn sort_primary(context: &TrackerContext, issue: &IssueProjection, sort: &str) -> String {
    match sort {
        "created" => format!("{:020}", issue.root_created_at),
        "updated" => format!("{:020}", context.event_time(&issue.tip_event)),
        "priority" => format!(
            "{}",
            match issue.priority {
                Some(Priority::P0) => 0,
                Some(Priority::P1) => 1,
                Some(Priority::P2) => 2,
                Some(Priority::P3) => 3,
                None => 4,
            }
        ),
        "due" => format!(
            "{:020}",
            issue.due.as_ref().map(|due| due.value).unwrap_or(u64::MAX)
        ),
        "identifier" => issue.identifier.to_lowercase(),
        _ => String::new(),
    }
}

async fn filter_issues<'a>(
    client: &BuzzClient,
    context: &'a TrackerContext,
    filters: &TrackFilterArgs,
) -> Result<Vec<&'a IssueProjection>, CliError> {
    if !matches!(
        filters.sort.as_str(),
        "created" | "updated" | "priority" | "due" | "identifier"
    ) {
        return Err(CliError::Usage(format!(
            "invalid --sort {:?}",
            filters.sort
        )));
    }
    if !matches!(filters.order.as_str(), "asc" | "desc") {
        return Err(CliError::Usage("--order must be asc or desc".into()));
    }
    let statuses: HashSet<IssueStatus> = filters
        .status
        .iter()
        .map(|value| parse_status(value))
        .collect::<Result<_, _>>()?;
    let priorities: HashSet<Option<Priority>> = filters
        .priority
        .iter()
        .map(|value| {
            if value == "none" {
                Ok(None)
            } else {
                parse_priority(value).map(Some)
            }
        })
        .collect::<Result<_, _>>()?;
    let channel_id = filters
        .channel
        .as_deref()
        .map(|value| {
            Uuid::parse_str(value)
                .map_err(|_| CliError::Usage(format!("invalid channel UUID: {value}")))
        })
        .transpose()?;
    let mut assignees = HashSet::new();
    let mut assignee_none = false;
    for value in &filters.assignee {
        if value == "none" {
            assignee_none = true;
        } else if value == "me" {
            assignees.insert(client.keys().public_key().to_hex());
        } else if let Ok(pubkey) = PublicKey::parse(value.trim_start_matches('@')) {
            assignees.insert(pubkey.to_hex());
        } else {
            let channel_id = channel_id.ok_or_else(|| {
                CliError::Usage("name-based --assignee filter requires --channel".into())
            })?;
            assignees.insert(resolve_assignee(client, channel_id, value).await?);
        }
    }
    let mut project_ids = HashSet::new();
    let mut project_none = false;
    for value in &filters.project {
        if value == "none" {
            project_none = true;
        } else {
            project_ids.insert(context.project(value)?.project_id);
        }
    }
    let due_before = filters
        .due_before
        .as_deref()
        .map(|value| parse_due(value, DEFAULT_TIMEZONE).map(|due| due.value))
        .transpose()?;
    let due_after = filters
        .due_after
        .as_deref()
        .map(|value| parse_due(value, DEFAULT_TIMEZONE).map(|due| due.value))
        .transpose()?;
    let updated_since = filters
        .updated_since
        .as_deref()
        .map(parse_timestamp)
        .transpose()?;
    let today = Utc::now().timestamp().max(0) as u64;
    let query = filters.query.as_ref().map(|value| value.to_lowercase());
    let required_labels: HashSet<String> = filters
        .label
        .iter()
        .map(|value| value.trim().to_lowercase())
        .collect();
    let any_labels: HashSet<String> = filters
        .label_any
        .iter()
        .map(|value| value.trim().to_lowercase())
        .collect();
    let mut items: Vec<_> = context
        .fold
        .issues
        .values()
        .filter(|issue| statuses.is_empty() || statuses.contains(&issue.status))
        .filter(|issue| {
            filters.include_closed || !statuses.is_empty() || !issue.status.is_terminal()
        })
        .filter(|issue| priorities.is_empty() || priorities.contains(&issue.priority))
        .filter(|issue| channel_id.is_none_or(|channel| issue.home_channel_id == Some(channel)))
        .filter(|issue| {
            (assignees.is_empty() && !assignee_none)
                || issue
                    .assignee
                    .as_ref()
                    .is_some_and(|value| assignees.contains(value))
                || (assignee_none && issue.assignee.is_none())
        })
        .filter(|issue| {
            (project_ids.is_empty() && !project_none)
                || issue
                    .project_id
                    .is_some_and(|value| project_ids.contains(&value))
                || (project_none && issue.project_id.is_none())
        })
        .filter(|issue| {
            required_labels
                .iter()
                .all(|label| issue.labels.contains(label))
        })
        .filter(|issue| {
            any_labels.is_empty() || any_labels.iter().any(|label| issue.labels.contains(label))
        })
        .filter(|issue| {
            due_before.is_none_or(|bound| issue.due.as_ref().is_some_and(|due| due.value < bound))
        })
        .filter(|issue| {
            due_after.is_none_or(|bound| issue.due.as_ref().is_some_and(|due| due.value > bound))
        })
        .filter(|issue| {
            !filters.overdue
                || (!issue.status.is_terminal()
                    && issue.due.as_ref().is_some_and(|due| due.value < today))
        })
        .filter(|issue| {
            query.as_ref().is_none_or(|needle| {
                issue.title.to_lowercase().contains(needle)
                    || issue.body.to_lowercase().contains(needle)
                    || issue.identifier.to_lowercase().contains(needle)
            })
        })
        .filter(|issue| {
            updated_since.is_none_or(|timestamp| context.event_time(&issue.tip_event) >= timestamp)
        })
        .collect();
    items.sort_by(|left, right| {
        let left_key = (
            sort_primary(context, left, &filters.sort),
            context.event_time(&left.tip_event),
            left.issue_id,
        );
        let right_key = (
            sort_primary(context, right, &filters.sort),
            context.event_time(&right.tip_event),
            right.issue_id,
        );
        let order = left_key.cmp(&right_key);
        if filters.order == "desc" {
            order.reverse()
        } else {
            order
        }
    });
    if let Some(cursor) = &filters.cursor {
        let cursor: ListCursor = decode_cursor(cursor)?;
        if cursor.sort != filters.sort {
            return Err(CliError::Usage(
                "cursor sort key does not match --sort".into(),
            ));
        }
        let position = items.iter().position(|issue| {
            sort_primary(context, issue, &filters.sort) == cursor.primary
                && context.event_time(&issue.tip_event) == cursor.updated_at
                && issue.issue_id == cursor.issue_id
        });
        if let Some(position) = position {
            items.drain(..=position);
        }
    }
    Ok(items)
}

fn page_issues(
    context: &TrackerContext,
    filters: &TrackFilterArgs,
    mut items: Vec<&IssueProjection>,
    active_tasks: &HashMap<Uuid, Uuid>,
    format: &OutputFormat,
) -> Result<Value, CliError> {
    let has_more = items.len() > filters.limit as usize;
    items.truncate(filters.limit as usize);
    let next_cursor = if has_more {
        items
            .last()
            .map(|issue| {
                encode_cursor(&ListCursor {
                    sort: filters.sort.clone(),
                    primary: sort_primary(context, issue, &filters.sort),
                    updated_at: context.event_time(&issue.tip_event),
                    issue_id: issue.issue_id,
                })
            })
            .transpose()?
    } else {
        None
    };
    let values: Vec<_> = items
        .into_iter()
        .map(|issue| {
            let value = issue_summary(context, issue, active_tasks.get(&issue.issue_id).copied());
            if matches!(format, OutputFormat::Compact) {
                compact_issue(value)
            } else {
                value
            }
        })
        .collect();
    Ok(json!({
        "items": values,
        "next_cursor": next_cursor,
        "snapshot_at": rfc3339(Utc::now().timestamp().max(0) as u64),
        "warnings": context.warnings,
    }))
}

async fn cmd_list(
    client: &BuzzClient,
    filters: TrackFilterArgs,
    format: &OutputFormat,
) -> Result<(), CliError> {
    let workspace_id = parse_workspace(filters.workspace.clone())?;
    let (context, _) = load_context(client, workspace_id).await?;
    let items = filter_issues(client, &context, &filters).await?;
    let tasks = task_projection(client).await?;
    let active_tasks = active_task_ids(&tasks);
    println!(
        "{}",
        page_issues(&context, &filters, items, &active_tasks, format)?
    );
    Ok(())
}

fn issue_activity(context: &TrackerContext, issue: &IssueProjection) -> Vec<Value> {
    let mut activity: Vec<_> = context
        .parsed
        .iter()
        .filter(|event| event.issue_id == issue.issue_id)
        .map(|event| {
            let (event_type, request_id, import) = match &event.payload {
                TrackerPayload::Root(root) => ("created", root.request_id, root.import.as_ref()),
                TrackerPayload::Revision(revision) => {
                    ("updated", revision.request_id, revision.import.as_ref())
                }
                TrackerPayload::Comment(comment) => (
                    "commented",
                    comment.content.request_id,
                    comment.content.import.as_ref(),
                ),
            };
            json!({
                "event_id": event.event_id,
                "type": event_type,
                "actor_pubkey": event.author_pubkey,
                "created_at": rfc3339(event.created_at),
                "request_id": request_id,
                "import_provenance": import,
                "dispatcher_suppressed": import.is_some(),
            })
        })
        .collect();
    activity.sort_by(|left, right| {
        (
            left.get("created_at").and_then(Value::as_str),
            left.get("event_id").and_then(Value::as_str),
        )
            .cmp(&(
                right.get("created_at").and_then(Value::as_str),
                right.get("event_id").and_then(Value::as_str),
            ))
    });
    activity
}

async fn task_projection(
    client: &BuzzClient,
) -> Result<BTreeMap<Uuid, TaskEnvelopeEvent>, CliError> {
    Ok(fold_task_events(query_task_events(client, None).await?))
}

fn active_task_ids(tasks: &BTreeMap<Uuid, TaskEnvelopeEvent>) -> HashMap<Uuid, Uuid> {
    let mut active = HashMap::<Uuid, (u64, String, Uuid)>::new();
    for event in tasks.values().filter(|event| {
        matches!(
            event.envelope.status,
            TaskStatus::Assigned | TaskStatus::PickedUp | TaskStatus::Blocked
        )
    }) {
        let Some(issue_id) = event.envelope.tracker_issue_id else {
            continue;
        };
        let candidate = (
            event.created_at,
            event.event_id.clone(),
            event.envelope.task_id,
        );
        if active.get(&issue_id).is_none_or(|prior| candidate > *prior) {
            active.insert(issue_id, candidate);
        }
    }
    active
        .into_iter()
        .map(|(issue_id, (_, _, task_id))| (issue_id, task_id))
        .collect()
}

fn linked_tasks(tasks: &BTreeMap<Uuid, TaskEnvelopeEvent>, issue_id: Uuid) -> Vec<Value> {
    tasks
        .values()
        .filter(|event| event.envelope.tracker_issue_id == Some(issue_id))
        .map(|event| {
            json!({
                "task_id": event.envelope.task_id,
                "status": event.envelope.status,
                "assignee": event.envelope.assignee,
                "event_id": event.event_id,
                "dispatch_request_id": event.envelope.dispatch_request_id,
            })
        })
        .collect()
}

async fn cmd_get(
    client: &BuzzClient,
    workspace: Option<String>,
    issue_key: String,
    no_history: bool,
    include_comments: bool,
    include_linked_tasks: bool,
    format: &OutputFormat,
) -> Result<(), CliError> {
    let workspace_id = parse_workspace(workspace)?;
    let (context, _) = load_context(client, workspace_id).await?;
    let issue = context.issue(&issue_key)?;
    let tasks = if include_linked_tasks || !no_history {
        task_projection(client).await?
    } else {
        BTreeMap::new()
    };
    let active_tasks = active_task_ids(&tasks);
    let mut output = issue_summary(&context, issue, active_tasks.get(&issue.issue_id).copied());
    let object = output
        .as_object_mut()
        .ok_or_else(|| CliError::Other("issue summary is not an object".into()))?;
    object.insert("body".into(), json!(issue.body));
    object.insert("relations".into(), json!(issue.relations));
    object.insert("external_refs".into(), json!(issue.external_refs));
    if !no_history {
        object.insert("activity".into(), json!(issue_activity(&context, issue)));
        if include_comments {
            object.insert(
                "comments".into(),
                json!(context
                    .fold
                    .comments
                    .get(&(workspace_id, issue.issue_id))
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|comment| json!({
                        "event_id": comment.event_id,
                        "author_pubkey": comment.author_pubkey,
                        "created_at": rfc3339(comment.created_at),
                        "reply_to": comment.parent_event,
                        "body": comment.body,
                        "request_id": comment.request_id,
                        "import": comment.import,
                    }))
                    .collect::<Vec<_>>()),
            );
        }
        if include_linked_tasks {
            object.insert(
                "linked_tasks".into(),
                json!(linked_tasks(&tasks, issue.issue_id)),
            );
        }
    }
    if matches!(format, OutputFormat::Compact) {
        output = compact_issue(output);
    }
    println!("{}", json!({"issue": output, "warnings": context.warnings}));
    Ok(())
}

async fn cmd_board(
    client: &BuzzClient,
    filters: TrackFilterArgs,
    group_by: String,
    columns: Option<String>,
    include_empty: bool,
    count_only: bool,
    format: &OutputFormat,
) -> Result<(), CliError> {
    if !matches!(
        group_by.as_str(),
        "status" | "priority" | "assignee" | "project"
    ) {
        return Err(CliError::Usage(
            "--group-by must be status, priority, assignee, or project".into(),
        ));
    }
    let workspace_id = parse_workspace(filters.workspace.clone())?;
    let (context, _) = load_context(client, workspace_id).await?;
    let items = filter_issues(client, &context, &filters).await?;
    let active_tasks = active_task_ids(&task_projection(client).await?);
    let mut groups: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    if let Some(columns) = columns {
        for column in columns
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            groups.entry(column.to_string()).or_default();
        }
    }
    for issue in items {
        let key = match group_by.as_str() {
            "status" => issue.status.to_string(),
            "priority" => issue
                .priority
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".into()),
            "assignee" => issue.assignee.clone().unwrap_or_else(|| "none".into()),
            "project" => issue
                .project_id
                .and_then(|id| {
                    context
                        .projects
                        .projects
                        .get(&id)
                        .map(|project| project.slug.clone())
                })
                .unwrap_or_else(|| "none".into()),
            _ => unreachable!(),
        };
        groups.entry(key).or_default().push(issue_summary(
            &context,
            issue,
            active_tasks.get(&issue.issue_id).copied(),
        ));
    }
    let groups: Vec<_> = groups
        .into_iter()
        .filter(|(_, items)| include_empty || !items.is_empty())
        .map(|(key, items)| {
            json!({
                "key": key,
                "count": items.len(),
                "items": if count_only { Vec::<Value>::new() } else { items },
            })
        })
        .collect();
    let _ = format;
    println!(
        "{}",
        json!({
            "group_by": group_by,
            "groups": groups,
            "next_cursor": null,
            "snapshot_at": rfc3339(Utc::now().timestamp().max(0) as u64),
            "warnings": context.warnings,
        })
    );
    Ok(())
}

async fn publish_revision(
    client: &BuzzClient,
    context: &mut TrackerContext,
    authorization: &TrackerAuthorization,
    revision: &IssueRevision,
) -> Result<(String, bool), CliError> {
    let request_id = revision
        .request_id
        .ok_or_else(|| CliError::Other("Tracker CLI revision lacks request_id".into()))?;
    let event = client.sign_event(
        build_tracker_revision(context.workspace_id, revision)
            .map_err(|error| usage("build Tracker revision", error))?,
    )?;
    if existing_request_event(client, context.workspace_id, request_id)
        .await?
        .is_some()
    {
        return submit_idempotent(client, context.workspace_id, request_id, event).await;
    }
    validate_candidate(context, authorization, &event, revision.issue_id)?;
    let (event_id, replayed) =
        submit_idempotent(client, context.workspace_id, request_id, event.clone()).await?;
    if !replayed {
        context.add_event(event, authorization);
        context.persist()?;
    }
    Ok((event_id, replayed))
}

#[allow(clippy::too_many_arguments)]
async fn cmd_comment(
    client: &BuzzClient,
    workspace: Option<String>,
    issue_key: String,
    content: String,
    reply_to: Option<String>,
    mentions: Vec<String>,
    request_id: Option<String>,
    import_json: Option<String>,
) -> Result<(), CliError> {
    let workspace_id = parse_workspace(workspace)?;
    let (mut context, authorization) = load_context(client, workspace_id).await?;
    let issue = context.issue(&issue_key)?.clone();
    let channel_id = issue
        .home_channel_id
        .ok_or_else(|| CliError::Usage("comment mentions require an issue home channel".into()))?;
    let mut resolved = Vec::new();
    for mention in mentions {
        resolved.push(resolve_issue_assignee(client, channel_id, &mention).await?);
    }
    let request_id = parse_request_id(request_id.as_deref())?;
    let comment = IssueComment {
        v: TRACKER_VERSION,
        issue_id: issue.issue_id,
        body: read_or_stdin(&content)?,
        request_id: Some(request_id),
        import: parse_import_json(import_json.as_deref())?,
    };
    let event = client.sign_event(
        build_tracker_comment(
            workspace_id,
            &issue.root_event,
            reply_to.as_deref(),
            &comment,
            &resolved,
        )
        .map_err(|error| usage("build Tracker comment", error))?,
    )?;
    if existing_request_event(client, workspace_id, request_id)
        .await?
        .is_some()
    {
        let (event_id, replayed) =
            submit_idempotent(client, workspace_id, request_id, event).await?;
        println!(
            "{}",
            json!({
                "accepted": true,
                "operation": "track.comment",
                "request_id": request_id,
                "event_ids": [event_id],
                "replayed": replayed,
                "comment": {
                    "event_id": event_id,
                    "issue_id": issue.issue_id,
                    "author_pubkey": client.keys().public_key().to_hex(),
                    "reply_to": reply_to,
                    "content": comment.body,
                },
                "warnings": context.warnings,
            })
        );
        return Ok(());
    }
    // Comments do not compete for the revision tip, but strict parsing and
    // membership fold validation still run before publication.
    let now = Utc::now().timestamp().max(0) as u64;
    let parsed =
        parse_tracker_event(&event, now).map_err(|error| usage("invalid comment", error))?;
    let mut candidate = context.parsed.clone();
    candidate.push(parsed);
    let folded = fold_tracker_events(candidate, &authorization);
    let comment_valid = folded
        .comments
        .get(&(workspace_id, issue.issue_id))
        .is_some_and(|comments| {
            comments
                .iter()
                .any(|comment| comment.event_id == event.id.to_hex())
        });
    if !comment_valid {
        return Err(CliError::Conflict(
            "comment would be rejected by the shared Tracker fold".into(),
        ));
    }
    let (event_id, replayed) =
        submit_idempotent(client, workspace_id, request_id, event.clone()).await?;
    if !replayed {
        context.add_event(event, &authorization);
        context.persist()?;
    }
    let author = client.keys().public_key().to_hex();
    println!(
        "{}",
        json!({
            "accepted": true,
            "operation": "track.comment",
            "request_id": request_id,
            "event_ids": [event_id],
            "replayed": replayed,
            "comment": {
                "event_id": event_id,
                "issue_id": issue.issue_id,
                "author_pubkey": author,
                "reply_to": reply_to,
                "content": comment.body,
            },
            "warnings": context.warnings,
        })
    );
    Ok(())
}

fn parse_relation_direction(value: &str) -> Result<RelationDirection, CliError> {
    match value {
        "blocks" => Ok(RelationDirection::Blocks),
        "blocked_by" => Ok(RelationDirection::BlockedBy),
        "duplicate_of" => Ok(RelationDirection::DuplicateOf),
        "parent" | "parent_of" => Ok(RelationDirection::Parent),
        "child" | "child_of" => Ok(RelationDirection::Child),
        "related_to" => Err(CliError::Usage(
            "related_to was removed by M12; use a typed directed relation".into(),
        )),
        _ => Err(CliError::Usage(format!("invalid relation type {value:?}"))),
    }
}

async fn cmd_relate(
    client: &BuzzClient,
    workspace: Option<String>,
    issue_key: String,
    relation_type: String,
    target_key: String,
    remove: bool,
    mutation: TrackMutationArgs,
) -> Result<(), CliError> {
    let workspace_id = parse_workspace(workspace)?;
    let (mut context, authorization) = load_context(client, workspace_id).await?;
    let issue = context.issue(&issue_key)?.clone();
    let target = context.issue(&target_key)?.clone();
    if mutation
        .if_revision
        .as_ref()
        .is_some_and(|expected| expected != &issue.tip_event)
    {
        return Err(CliError::Conflict(format!(
            "current revision is {}; expected {}",
            issue.tip_event,
            mutation.if_revision.as_deref().unwrap_or_default()
        )));
    }
    let direction = parse_relation_direction(&relation_type)?;
    let canonical = canonicalize_relation(
        issue.issue_id,
        issue.root_event.clone(),
        direction,
        target.issue_id,
        target.root_event.clone(),
    );
    let source = context
        .issue(&canonical.source_issue_id.to_string())?
        .clone();
    let request_id = parse_request_id(mutation.request_id.as_deref())?;
    let mut relations = SetOperations::default();
    if remove {
        relations.remove.push(canonical.relation.clone());
    } else {
        relations.add.push(canonical.relation.clone());
    }
    let revision = IssueRevision {
        v: TRACKER_VERSION,
        op: "revise".into(),
        issue_id: source.issue_id,
        root_event: source.root_event.clone(),
        parent_event: source.tip_event.clone(),
        revision: context.revision_number(&source) + 1,
        set: RevisionSet::default(),
        clear: vec![],
        relations,
        external_refs: SetOperations::default(),
        reason: read_optional_stdin(mutation.reason)?,
        request_id: Some(request_id),
        import: parse_import_json(mutation.import_json.as_deref())?,
    };
    let (event_id, replayed) =
        publish_revision(client, &mut context, &authorization, &revision).await?;
    println!(
        "{}",
        json!({
            "accepted": true,
            "operation": "track.relate",
            "request_id": request_id,
            "event_ids": [event_id],
            "replayed": replayed,
            "relation": {
                "source_issue_id": issue.issue_id,
                "type": relation_type,
                "target_issue_id": target.issue_id,
                "active": !remove,
                "canonical_source_issue_id": canonical.source_issue_id,
            },
            "warnings": context.warnings,
        })
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_link(
    client: &BuzzClient,
    workspace: Option<String>,
    issue_key: String,
    link_type: String,
    reference: String,
    title: Option<String>,
    remove: bool,
    mutation: TrackMutationArgs,
) -> Result<(), CliError> {
    let workspace_id = parse_workspace(workspace)?;
    let (mut context, authorization) = load_context(client, workspace_id).await?;
    let issue = context.issue(&issue_key)?.clone();
    if mutation
        .if_revision
        .as_ref()
        .is_some_and(|expected| expected != &issue.tip_event)
    {
        return Err(CliError::Conflict(format!(
            "current revision is {}; expected {}",
            issue.tip_event,
            mutation.if_revision.as_deref().unwrap_or_default()
        )));
    }
    if link_type.trim().is_empty() || reference.trim().is_empty() {
        return Err(CliError::Usage("link type/ref cannot be empty".into()));
    }
    let external = ExternalRef {
        kind: link_type.clone(),
        reference: reference.clone(),
    };
    let request_id = parse_request_id(mutation.request_id.as_deref())?;
    let mut external_refs = SetOperations::default();
    if remove {
        external_refs.remove.push(external);
    } else {
        external_refs.add.push(external);
    }
    let revision = IssueRevision {
        v: TRACKER_VERSION,
        op: "revise".into(),
        issue_id: issue.issue_id,
        root_event: issue.root_event.clone(),
        parent_event: issue.tip_event.clone(),
        revision: context.revision_number(&issue) + 1,
        set: RevisionSet::default(),
        clear: vec![],
        relations: SetOperations::default(),
        external_refs,
        reason: read_optional_stdin(mutation.reason)?,
        request_id: Some(request_id),
        import: parse_import_json(mutation.import_json.as_deref())?,
    };
    let (event_id, replayed) =
        publish_revision(client, &mut context, &authorization, &revision).await?;
    let mut warnings = context.warnings.clone();
    if title.is_some() {
        warnings.push(
            "link titles are display-only in P0 and are not stored in the typed ref identity"
                .into(),
        );
    }
    println!(
        "{}",
        json!({
            "accepted": true,
            "operation": "track.link",
            "request_id": request_id,
            "event_ids": [event_id],
            "replayed": replayed,
            "link": {"type": link_type, "ref": reference, "title": title, "active": !remove},
            "warnings": warnings,
        })
    );
    Ok(())
}

async fn task_for_dispatch_request(
    client: &BuzzClient,
    request_id: Uuid,
) -> Result<Option<TaskEnvelopeEvent>, CliError> {
    let current = fold_task_events(query_task_events(client, None).await?);
    let mut matches = current
        .into_values()
        .filter(|event| event.envelope.dispatch_request_id == Some(request_id));
    let found = matches.next();
    if matches.next().is_some() {
        return Err(CliError::Conflict(format!(
            "dispatch request {request_id} names multiple tasks"
        )));
    }
    Ok(found)
}

#[allow(clippy::too_many_arguments)]
async fn cmd_dispatch(
    client: &BuzzClient,
    workspace: Option<String>,
    issue_key: String,
    assignee: String,
    channel: Option<String>,
    title: Option<String>,
    deliverables: Vec<String>,
    verification: Option<String>,
    content: Option<String>,
    replace_active: bool,
    set_status: String,
    mutation: TrackMutationArgs,
) -> Result<(), CliError> {
    let workspace_id = parse_workspace(workspace)?;
    let import = parse_import_json(mutation.import_json.as_deref())?;
    let reason = read_optional_stdin(mutation.reason.clone())?;
    let (mut context, authorization) = load_context(client, workspace_id).await?;
    let issue = context.issue(&issue_key)?.clone();
    if issue.status.is_terminal() {
        return Err(CliError::Conflict(
            "terminal issues must be reopened before dispatch".into(),
        ));
    }
    if mutation
        .if_revision
        .as_ref()
        .is_some_and(|expected| expected != &issue.tip_event)
    {
        return Err(CliError::Conflict(format!(
            "current revision is {}; expected {}",
            issue.tip_event,
            mutation.if_revision.as_deref().unwrap_or_default()
        )));
    }
    let request_id = parse_request_id(mutation.request_id.as_deref())?;
    let channel_id = match channel {
        Some(value) => Uuid::parse_str(&value)
            .map_err(|_| CliError::Usage(format!("invalid channel UUID: {value}")))?,
        None => issue.home_channel_id.ok_or_else(|| {
            CliError::Usage("--channel is required when the issue has no home channel".into())
        })?,
    };
    let assignee = resolve_issue_assignee(client, channel_id, &assignee).await?;
    let set_status = parse_status(&set_status)?;
    if set_status.is_terminal() {
        return Err(CliError::Usage(
            "dispatch --set-status cannot set a terminal state".into(),
        ));
    }
    let project_default = issue
        .project_id
        .and_then(|id| context.projects.projects.get(&id))
        .and_then(|project| project.default_verify.clone());
    let verification = verification.or(project_default).ok_or_else(|| {
        CliError::Usage("--verify is required unless the project supplies --default-verify".into())
    })?;
    let deliverables = parse_deliverables(&deliverables)?;
    let task_title = title
        .clone()
        .unwrap_or_else(|| format!("{}: {}", issue.identifier, issue.title));
    let existing_task = task_for_dispatch_request(client, request_id).await?;
    let current_tasks = fold_task_events(query_task_events(client, None).await?);
    let active: Vec<_> = current_tasks
        .values()
        .filter(|event| event.envelope.tracker_issue_id == Some(issue.issue_id))
        .filter(|event| event.envelope.dispatch_request_id != Some(request_id))
        .filter(|event| {
            !matches!(
                event.envelope.status,
                TaskStatus::Done | TaskStatus::Failed | TaskStatus::Verified
            )
        })
        .collect();
    if !active.is_empty() && !replace_active {
        return Err(CliError::Conflict(format!(
            "issue has {} active linked task(s); pass --replace-active",
            active.len()
        )));
    }

    let (task_event_id, task_id, task_replayed) = if let Some(existing) = existing_task {
        if existing.envelope.tracker_issue_id != Some(issue.issue_id)
            || existing.envelope.assignee != assignee
            || existing.envelope.verification.method != verification
            || existing.envelope.deliverables != deliverables
            || existing.envelope.title != task_title
            || existing.channel_id != channel_id
        {
            return Err(CliError::Conflict(format!(
                "dispatch request {request_id} was already used with different semantic input"
            )));
        }
        (existing.event_id, existing.envelope.task_id, true)
    } else {
        let task_id = Uuid::new_v4();
        let envelope = TaskEnvelope {
            v: TASK_ENVELOPE_VERSION,
            task_id,
            status: TaskStatus::Assigned,
            assignee: assignee.clone(),
            assigner: client.keys().public_key().to_hex(),
            title: task_title,
            deliverables,
            verification: TaskVerification {
                method: verification,
                evidence_ref: None,
            },
            blocked_on: None,
            notes: None,
            tracker_issue_id: Some(issue.issue_id),
            dispatch_request_id: Some(request_id),
        }
        .validate()
        .map_err(|error| usage("invalid dispatch task", error))?;
        let human = match content {
            Some(value) => read_or_stdin(&value)?,
            None => format!("Dispatch {} for execution.", issue.identifier),
        };
        let builder = buzz_sdk::task_envelope::build_task_message(
            channel_id,
            &human,
            &envelope,
            None,
            &[assignee.as_str()],
        )
        .map_err(|error| usage("build task assignment", error))?;
        let event = client.sign_event(builder)?;
        let raw = client.submit_event(event.clone()).await?;
        let response: Value = serde_json::from_str(&normalize_write_response(&raw))
            .map_err(|error| CliError::Other(format!("invalid task receipt: {error}")))?;
        if response.get("accepted").and_then(Value::as_bool) != Some(true) {
            return Err(CliError::Other(format!(
                "relay rejected task assignment: {}",
                response
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown reason")
            )));
        }
        (event.id.to_hex(), task_id, false)
    };

    let already_linked = issue
        .external_refs
        .iter()
        .any(|value| value.kind == "task" && value.reference == task_id.to_string());
    let (link_event_id, link_replayed) = if already_linked {
        (issue.tip_event.clone(), true)
    } else {
        let mut set = RevisionSet {
            assignee: Some(assignee.clone()),
            ..RevisionSet::default()
        };
        if set_status != issue.status {
            set.status = Some(set_status);
        }
        let revision = IssueRevision {
            v: TRACKER_VERSION,
            op: "revise".into(),
            issue_id: issue.issue_id,
            root_event: issue.root_event.clone(),
            parent_event: issue.tip_event.clone(),
            revision: context.revision_number(&issue) + 1,
            set,
            clear: vec![],
            relations: SetOperations::default(),
            external_refs: SetOperations {
                add: vec![
                    ExternalRef {
                        kind: "task".into(),
                        reference: task_id.to_string(),
                    },
                    ExternalRef {
                        kind: "task_event".into(),
                        reference: task_event_id.clone(),
                    },
                ],
                remove: vec![],
            },
            reason: if set_status != issue.status {
                Some(reason.unwrap_or_else(|| "dispatched for execution".into()))
            } else {
                reason
            },
            request_id: Some(request_id),
            import,
        };
        match publish_revision(client, &mut context, &authorization, &revision).await {
            Ok(result) => result,
            Err(error) => {
                println!(
                    "{}",
                    json!({
                        "accepted": false,
                        "partial": true,
                        "operation": "track.dispatch",
                        "request_id": request_id,
                        "completed": [{"role": "task_assignment", "event_id": task_event_id}],
                        "pending": ["issue_task_link"],
                        "issue_id": issue.issue_id,
                        "task_id": task_id,
                        "retryable": true,
                        "message": "task assigned; reciprocal issue link was not accepted; retry with the same --request-id",
                    })
                );
                return Err(error);
            }
        }
    };
    println!(
        "{}",
        json!({
            "accepted": true,
            "operation": "track.dispatch",
            "request_id": request_id,
            "event_ids": [task_event_id, link_event_id],
            "issue_id": issue.issue_id,
            "task_id": task_id,
            "task_event_id": task_event_id,
            "issue_link_event_id": link_event_id,
            "assignee_pubkey": assignee,
            "issue_revision_event_id": link_event_id,
            "replayed": task_replayed && link_replayed,
            "warnings": context.warnings,
        })
    );
    Ok(())
}

fn make_tag(parts: &[&str]) -> Result<Tag, CliError> {
    Tag::parse(parts.iter().copied())
        .map_err(|error| CliError::Other(format!("tag construction failed: {error}")))
}

fn parse_project_statuses(values: &[String]) -> Result<Vec<IssueStatus>, CliError> {
    let statuses = if values.is_empty() {
        default_project_statuses()
    } else {
        values
            .iter()
            .map(|value| parse_status(value))
            .collect::<Result<Vec<_>, _>>()?
    };
    let unique: HashSet<_> = statuses.iter().copied().collect();
    if unique.len() != statuses.len() {
        return Err(CliError::Usage(
            "project --status values must be unique".into(),
        ));
    }
    Ok(statuses)
}

fn project_json(project: &TrackerProject) -> Value {
    json!({
        "project_id": project.project_id,
        "name": project.name,
        "slug": project.slug,
        "statuses": project.statuses,
        "default_channel_id": project.default_channel_id,
        "default_verify": project.default_verify,
        "revision_event_id": project.revision_event_id,
        "updated_at": rfc3339(project.updated_at),
    })
}

async fn submit_raw_event(client: &BuzzClient, event: Event) -> Result<String, CliError> {
    let raw = client.submit_event(event.clone()).await?;
    let response: Value = serde_json::from_str(&normalize_write_response(&raw))
        .map_err(|error| CliError::Other(format!("invalid relay write receipt: {error}")))?;
    if response.get("accepted").and_then(Value::as_bool) != Some(true) {
        return Err(CliError::Other(format!(
            "relay rejected event: {}",
            response
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown reason")
        )));
    }
    Ok(event.id.to_hex())
}

#[allow(clippy::too_many_arguments)]
fn tracker_project_tags(
    workspace_id: Uuid,
    project_id: Uuid,
    slug: &str,
    name: &str,
    statuses: &[IssueStatus],
    default_channel_id: Option<Uuid>,
    default_verify: Option<&str>,
    preserved: impl IntoIterator<Item = Tag>,
) -> Result<Vec<Tag>, CliError> {
    if slug.trim().is_empty() || slug.len() > 1024 {
        return Err(CliError::Usage(
            "project slug must contain 1..=1024 bytes".into(),
        ));
    }
    if name.trim().is_empty() || name.len() > 256 {
        return Err(CliError::Usage(
            "project name must contain 1..=256 bytes".into(),
        ));
    }
    let replaced: HashSet<&str> = [
        "auth",
        "d",
        "name",
        "buzz-project-id",
        "buzz-workspace",
        "buzz-status",
        "buzz-channel",
        "buzz-default-verify",
        PROJECT_UPDATE_INTENT_TAG,
        "request",
    ]
    .into_iter()
    .collect();
    let mut tags: Vec<Tag> = preserved
        .into_iter()
        .filter(|tag| {
            tag.as_slice()
                .first()
                .is_none_or(|name| !replaced.contains(name.as_str()))
        })
        .collect();
    tags.extend([
        make_tag(&["d", slug])?,
        make_tag(&["name", name])?,
        make_tag(&["buzz-project-id", &project_id.to_string()])?,
        make_tag(&["buzz-workspace", &workspace_id.to_string()])?,
    ]);
    for status in statuses {
        tags.push(make_tag(&["buzz-status", &status.to_string()])?);
    }
    if let Some(channel_id) = default_channel_id {
        tags.push(make_tag(&["buzz-channel", &channel_id.to_string()])?);
    }
    if let Some(default_verify) = default_verify {
        if default_verify.trim().is_empty() || default_verify.len() > 2048 {
            return Err(CliError::Usage(
                "default verification must contain 1..=2048 bytes".into(),
            ));
        }
        tags.push(make_tag(&["buzz-default-verify", default_verify])?);
    }
    Ok(tags)
}

#[allow(clippy::too_many_arguments)]
async fn cmd_project_create(
    client: &BuzzClient,
    workspace: Option<String>,
    requested_project_id: Option<String>,
    name: String,
    slug: String,
    status: Vec<String>,
    default_channel: Option<String>,
    default_verify: Option<String>,
    mutation: TrackMutationArgs,
) -> Result<(), CliError> {
    let workspace_id = parse_workspace(workspace)?;
    let (mut context, authorization) = load_context(client, workspace_id).await?;
    let request_id = parse_request_id(mutation.request_id.as_deref())?;
    if let Some(existing) = existing_request_event(client, workspace_id, request_id).await? {
        if existing.kind.as_u16() as u32 == KIND_TRACKER_PROJECT_ACTIVITY {
            let activity = buzz_sdk::tracker::parse_tracker_project_activity(
                &existing,
                Utc::now().timestamp().max(0) as u64,
            )
            .map_err(|error| usage("parse existing project activity", error))?;
            let project = context.project(&activity.activity.project_id.to_string())?;
            println!(
                "{}",
                json!({
                    "accepted": true,
                    "operation": "track.project.create",
                    "request_id": request_id,
                    "event_ids": [project.revision_event_id, existing.id.to_hex()],
                    "replayed": true,
                    "project": project_json(project),
                    "warnings": context.warnings,
                })
            );
            return Ok(());
        }
    }
    let project_id = requested_project_id
        .as_deref()
        .map(|value| {
            Uuid::parse_str(value)
                .map_err(|_| CliError::Usage(format!("invalid project UUID: {value}")))
        })
        .transpose()?
        .unwrap_or(request_id);
    let statuses = parse_project_statuses(&status)?;
    let default_channel_id = default_channel
        .as_deref()
        .map(|value| {
            Uuid::parse_str(value)
                .map_err(|_| CliError::Usage(format!("invalid channel UUID: {value}")))
        })
        .transpose()?;
    let existing_head = context
        .projects
        .projects
        .values()
        .find(|project| project.slug == slug)
        .cloned();
    if let Some(existing) = &existing_head {
        if existing.project_id != project_id
            || existing.name != name
            || existing.statuses != statuses
            || existing.default_channel_id != default_channel_id
            || existing.default_verify != default_verify
        {
            return Err(CliError::Conflict(format!(
                "project slug {slug:?} already exists with different semantic input"
            )));
        }
    }
    let tags = tracker_project_tags(
        workspace_id,
        project_id,
        &slug,
        &name,
        &statuses,
        default_channel_id,
        default_verify.as_deref(),
        [],
    )?;
    let head = client.sign_event(
        build_project_with_tags("", tags)
            .map_err(|error| usage("build Tracker project head", error))?,
    )?;
    let (project, head_id) = if let Some(existing) = existing_head {
        let head_id = existing.revision_event_id.clone();
        (existing, head_id)
    } else {
        let head_id = submit_raw_event(client, head.clone()).await?;
        let project = parse_tracker_project(&head)
            .map_err(|error| usage("validate Tracker project head", error))?;
        (project, head_id)
    };
    let activity = ProjectActivity {
        v: TRACKER_VERSION,
        op: ProjectActivityOperation::Create,
        project_id,
        request_id,
        before: None,
        after: TrackerProjectSnapshot::from(&project),
    };
    let activity_event = client.sign_event(
        build_tracker_project_activity(
            workspace_id,
            &activity,
            &client.keys().public_key().to_hex(),
        )
        .map_err(|error| usage("build project activity", error))?,
    )?;
    let activity_result =
        submit_idempotent(client, workspace_id, request_id, activity_event.clone()).await;
    let (activity_id, replayed) = match activity_result {
        Ok(result) => result,
        Err(error) => {
            println!(
                "{}",
                json!({
                    "accepted": false,
                    "partial": true,
                    "operation": "track.project.create",
                    "request_id": request_id,
                    "completed": [{"role": "project_head", "event_id": head_id}],
                    "pending": ["project_activity"],
                    "project_id": project_id,
                    "retryable": true,
                })
            );
            return Err(error);
        }
    };
    if !replayed {
        context.add_event(activity_event, &authorization);
        context.persist()?;
    }
    println!(
        "{}",
        json!({
            "accepted": true,
            "operation": "track.project.create",
            "request_id": request_id,
            "event_ids": [head_id, activity_id],
            "replayed": false,
            "project": project_json(&project),
            "warnings": context.warnings,
        })
    );
    Ok(())
}

async fn fetch_project_head(
    client: &BuzzClient,
    project: &TrackerProject,
) -> Result<Event, CliError> {
    let events = client
        .query_paginated(
            json!({
                "kinds": [KIND_PROJECT],
                "authors": [project.author_pubkey],
                "#d": [project.slug],
            }),
            1,
        )
        .await?;
    let value = events
        .into_iter()
        .next()
        .ok_or_else(|| CliError::NotFound("current project head disappeared".into()))?;
    serde_json::from_value(value)
        .map_err(|error| CliError::Other(format!("invalid project head JSON: {error}")))
}

fn derived_request_id(request_id: Uuid, scope: &str) -> Uuid {
    let mut digest = Sha256::new();
    digest.update(request_id.as_bytes());
    digest.update([0]);
    digest.update(scope.as_bytes());
    let digest = digest.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

async fn project_update_head_by_request(
    client: &BuzzClient,
    request_id: Uuid,
) -> Result<Option<Event>, CliError> {
    let events = client
        .query_paginated(
            json!({
                "kinds": [KIND_PROJECT],
                "authors": [client.keys().public_key().to_hex()],
                "#request": [request_id.to_string()],
            }),
            2,
        )
        .await?;
    if events.len() > 1 {
        return Err(CliError::Conflict(format!(
            "project update request {request_id} names multiple heads"
        )));
    }
    events
        .into_iter()
        .next()
        .map(|value| {
            serde_json::from_value(value)
                .map_err(|error| CliError::Other(format!("invalid project head JSON: {error}")))
        })
        .transpose()
}

fn project_update_intent(event: &Event) -> Result<ProjectUpdateIntent, CliError> {
    let values: Vec<_> = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some(PROJECT_UPDATE_INTENT_TAG))
        .collect();
    if values.len() != 1 {
        return Err(CliError::Conflict(
            "Tracker project update head must carry exactly one update intent".into(),
        ));
    }
    let raw = values[0]
        .as_slice()
        .get(1)
        .ok_or_else(|| CliError::Conflict("Tracker project update intent is empty".into()))?;
    serde_json::from_str(raw)
        .map_err(|error| CliError::Conflict(format!("invalid project update intent: {error}")))
}

async fn submit_project_delete(
    client: &BuzzClient,
    request_id: Uuid,
    candidate: Event,
) -> Result<(String, bool), CliError> {
    let events = client
        .query_paginated(
            json!({
                "kinds": [5],
                "authors": [client.keys().public_key().to_hex()],
                "#request": [request_id.to_string()],
            }),
            2,
        )
        .await?;
    if events.len() > 1 {
        return Err(CliError::Conflict(format!(
            "project delete request {request_id} names multiple events"
        )));
    }
    if let Some(existing) = events.into_iter().next() {
        let existing: Event = serde_json::from_value(existing)
            .map_err(|error| CliError::Other(format!("invalid deletion event JSON: {error}")))?;
        if same_semantic_request(&existing, &candidate) {
            return Ok((existing.id.to_hex(), true));
        }
        return Err(CliError::Conflict(format!(
            "project delete request {request_id} was already used with different semantic input"
        )));
    }
    submit_raw_event(client, candidate)
        .await
        .map(|event_id| (event_id, false))
}

fn parse_status_maps(values: &[String]) -> Result<HashMap<IssueStatus, IssueStatus>, CliError> {
    let mut maps = HashMap::new();
    for value in values {
        let (from, to) = value.split_once('=').ok_or_else(|| {
            CliError::Usage(format!("--map-status must be old=new (got {value:?})"))
        })?;
        if maps
            .insert(parse_status(from)?, parse_status(to)?)
            .is_some()
        {
            return Err(CliError::Usage(format!(
                "duplicate --map-status source {from:?}"
            )));
        }
    }
    Ok(maps)
}

#[allow(clippy::too_many_arguments)]
async fn cmd_project_update(
    client: &BuzzClient,
    workspace: Option<String>,
    project_key: String,
    name: Option<String>,
    slug: Option<String>,
    status: Vec<String>,
    default_channel: Option<String>,
    default_verify: Option<String>,
    map_status: Vec<String>,
    mutation: TrackMutationArgs,
) -> Result<(), CliError> {
    let workspace_id = parse_workspace(workspace)?;
    let (mut context, authorization) = load_context(client, workspace_id).await?;
    let request_id = parse_request_id(mutation.request_id.as_deref())?;
    let reason = read_optional_stdin(mutation.reason.clone())?
        .unwrap_or_else(|| "project workflow update".into());
    let existing_activity = existing_request_event(client, workspace_id, request_id).await?;
    if existing_activity
        .as_ref()
        .is_some_and(|event| event.kind.as_u16() as u32 != KIND_TRACKER_PROJECT_ACTIVITY)
    {
        return Err(CliError::Conflict(format!(
            "request {request_id} was already used by another Tracker mutation"
        )));
    }
    let existing_head = project_update_head_by_request(client, request_id).await?;
    if existing_activity.is_some() && existing_head.is_none() {
        return Err(CliError::Conflict(format!(
            "request {request_id} already names project activity without its matching update head"
        )));
    }
    let existing_intent = existing_head
        .as_ref()
        .map(project_update_intent)
        .transpose()?;
    let initial_current = if existing_intent.is_none() {
        Some(context.project(&project_key)?.clone())
    } else {
        None
    };
    let before = if let Some(intent) = &existing_intent {
        intent.before.clone()
    } else {
        TrackerProjectSnapshot::from(initial_current.as_ref().ok_or_else(|| {
            CliError::Other("project update lost both current project and retry intent".into())
        })?)
    };
    let key_matches = project_key == before.project_id.to_string()
        || project_key == before.slug
        || existing_head.as_ref().is_some_and(|event| {
            parse_tracker_project(event).is_ok_and(|project| project.slug == project_key)
        });
    if !key_matches {
        return Err(CliError::Conflict(format!(
            "project update request {request_id} belongs to project {} ({})",
            before.project_id, before.slug
        )));
    }
    if mutation
        .if_revision
        .as_ref()
        .is_some_and(|expected| expected != &before.revision_event_id)
    {
        return Err(CliError::Conflict(format!(
            "project revision at request start was {}; expected {}",
            before.revision_event_id,
            mutation.if_revision.as_deref().unwrap_or_default()
        )));
    }
    let next_name = name.unwrap_or_else(|| before.name.clone());
    let next_slug = slug.unwrap_or_else(|| before.slug.clone());
    if next_slug != before.slug
        && context
            .projects
            .projects
            .values()
            .any(|project| project.project_id != before.project_id && project.slug == next_slug)
    {
        return Err(CliError::Conflict(format!(
            "project slug {next_slug:?} already exists"
        )));
    }
    let next_statuses = if status.is_empty() {
        before.statuses.clone()
    } else {
        parse_project_statuses(&status)?
    };
    let next_channel = match default_channel.as_deref() {
        Some("none") => None,
        Some(value) => Some(
            Uuid::parse_str(value)
                .map_err(|_| CliError::Usage(format!("invalid channel UUID: {value}")))?,
        ),
        None => before.default_channel_id,
    };
    let next_verify = match default_verify.as_deref() {
        Some("none") => None,
        Some(value) => Some(value.to_string()),
        None => before.default_verify.clone(),
    };
    let maps = parse_status_maps(&map_status)?;
    let mut status_maps: Vec<_> = maps.iter().map(|(from, to)| (*from, *to)).collect();
    status_maps.sort_by_key(|(from, to)| (from.to_string(), to.to_string()));
    let removed: HashSet<_> = before
        .statuses
        .iter()
        .copied()
        .filter(|value| !next_statuses.contains(value))
        .collect();
    let migrations = if let Some(intent) = &existing_intent {
        if intent.v != TRACKER_VERSION
            || intent.request_id != request_id
            || intent.reason != reason
            || intent.status_maps != status_maps
        {
            return Err(CliError::Conflict(format!(
                "project update request {request_id} was already used with different semantic input"
            )));
        }
        intent.migrations.clone()
    } else {
        let affected: Vec<_> = context
            .fold
            .issues
            .values()
            .filter(|issue| {
                issue.project_id == Some(before.project_id) && removed.contains(&issue.status)
            })
            .cloned()
            .collect();
        let mut migrations = Vec::with_capacity(affected.len());
        for issue in affected {
            let mapped = maps.get(&issue.status).copied().ok_or_else(|| {
                CliError::Conflict(format!(
                    "status {} is used by live issue {}; pass --map-status {}=<replacement>",
                    issue.status, issue.identifier, issue.status
                ))
            })?;
            if !next_statuses.contains(&mapped) {
                return Err(CliError::Usage(format!(
                    "mapped status {mapped} is not present in the new project workflow"
                )));
            }
            let revision = context.revision_number(&issue) + 1;
            migrations.push(ProjectStatusMigration {
                issue_id: issue.issue_id,
                root_event: issue.root_event,
                parent_event: issue.tip_event,
                revision,
                from: issue.status,
                to: mapped,
            });
        }
        migrations.sort_by_key(|migration| migration.issue_id);
        migrations
    };
    let intent = ProjectUpdateIntent {
        v: TRACKER_VERSION,
        request_id,
        before: before.clone(),
        reason: reason.clone(),
        status_maps,
        migrations,
    };

    let (new_head, new_head_id, head_replayed) = if let Some(event) = existing_head {
        let project = parse_tracker_project(&event)
            .map_err(|error| usage("validate existing Tracker project head", error))?;
        if project.project_id != before.project_id
            || project.name != next_name
            || project.slug != next_slug
            || project.statuses != next_statuses
            || project.default_channel_id != next_channel
            || project.default_verify != next_verify
            || existing_intent.as_ref() != Some(&intent)
        {
            return Err(CliError::Conflict(format!(
                "project update request {request_id} was already used with different semantic input"
            )));
        }
        let event_id = event.id.to_hex();
        (event, event_id, true)
    } else {
        let current = initial_current.as_ref().ok_or_else(|| {
            CliError::Other("project update lost current project before head publish".into())
        })?;
        let old_head = fetch_project_head(client, current).await?;
        let mut tags = tracker_project_tags(
            workspace_id,
            before.project_id,
            &next_slug,
            &next_name,
            &next_statuses,
            next_channel,
            next_verify.as_deref(),
            old_head.tags.iter().cloned(),
        )?;
        tags.push(make_tag(&["request", &request_id.to_string()])?);
        tags.push(make_tag(&[
            PROJECT_UPDATE_INTENT_TAG,
            &serde_json::to_string(&intent).map_err(|error| {
                CliError::Other(format!("serialize project update intent: {error}"))
            })?,
        ])?);
        let next_timestamp = old_head.created_at.as_secs().saturating_add(1);
        let event = client.sign_event(
            build_project_with_tags(&old_head.content, tags)
                .map_err(|error| usage("build Tracker project head", error))?
                .custom_created_at(Timestamp::from(next_timestamp)),
        )?;
        match submit_raw_event(client, event.clone()).await {
            Ok(event_id) => (event, event_id, false),
            Err(error) => {
                println!(
                    "{}",
                    json!({
                        "accepted": false,
                        "partial": true,
                        "operation": "track.project.update",
                        "request_id": request_id,
                        "completed": [],
                        "pending": ["project_head", "issue_status_migrations", "delete_old_project_address", "project_activity"],
                        "project_id": before.project_id,
                        "retryable": true,
                    })
                );
                return Err(error);
            }
        }
    };
    let next_project = parse_tracker_project(&new_head)
        .map_err(|error| usage("validate Tracker project head", error))?;
    let activity = ProjectActivity {
        v: TRACKER_VERSION,
        op: if next_slug == before.slug {
            ProjectActivityOperation::Update
        } else {
            ProjectActivityOperation::Rename
        },
        project_id: before.project_id,
        request_id,
        before: Some(before.clone()),
        after: TrackerProjectSnapshot::from(&next_project),
    };
    if let Some(existing) = existing_activity {
        let parsed = buzz_sdk::tracker::parse_tracker_project_activity(
            &existing,
            Utc::now().timestamp().max(0) as u64,
        )
        .map_err(|error| usage("parse existing project activity", error))?;
        if parsed.activity != activity {
            return Err(CliError::Conflict(format!(
                "project update request {request_id} was already used with different semantic input"
            )));
        }
        println!(
            "{}",
            json!({
                "accepted": true,
                "operation": "track.project.update",
                "request_id": request_id,
                "event_ids": [new_head_id, existing.id.to_hex()],
                "replayed": true,
                "project": project_json(&next_project),
                "warnings": context.warnings,
            })
        );
        return Ok(());
    }

    let mut event_ids = vec![new_head_id.clone()];
    let mut all_replayed = head_replayed;
    for migration in &intent.migrations {
        let migration_request =
            derived_request_id(request_id, &format!("status:{}", migration.issue_id));
        let revision = IssueRevision {
            v: TRACKER_VERSION,
            op: "revise".into(),
            issue_id: migration.issue_id,
            root_event: migration.root_event.clone(),
            parent_event: migration.parent_event.clone(),
            revision: migration.revision,
            set: RevisionSet {
                status: Some(migration.to),
                ..RevisionSet::default()
            },
            clear: vec![],
            relations: SetOperations::default(),
            external_refs: SetOperations::default(),
            reason: Some(reason.clone()),
            request_id: Some(migration_request),
            import: None,
        };
        let (event_id, replayed) = match publish_revision(
            client,
            &mut context,
            &authorization,
            &revision,
        )
        .await
        {
            Ok(result) => result,
            Err(error) => {
                println!(
                    "{}",
                    json!({
                        "accepted": false,
                        "partial": true,
                        "operation": "track.project.update",
                        "request_id": request_id,
                        "completed_event_ids": event_ids,
                        "pending": ["issue_status_migrations", "delete_old_project_address", "project_activity"],
                        "failed_issue_id": migration.issue_id,
                        "project_id": before.project_id,
                        "retryable": true,
                    })
                );
                return Err(error);
            }
        };
        event_ids.push(event_id);
        all_replayed &= replayed;
    }
    if next_slug != before.slug {
        let delete_request = derived_request_id(request_id, "delete-old-project-address");
        let delete = client.sign_event(
            build_delete_addressable(
                KIND_PROJECT,
                &client.keys().public_key().to_hex(),
                &before.slug,
            )
            .map_err(|error| usage("build old project address deletion", error))?
            .tag(make_tag(&["request", &delete_request.to_string()])?)
            .custom_created_at(Timestamp::from(
                new_head.created_at.as_secs().saturating_add(1),
            )),
        )?;
        match submit_project_delete(client, delete_request, delete).await {
            Ok((event_id, replayed)) => {
                event_ids.push(event_id);
                all_replayed &= replayed;
            }
            Err(error) => {
                println!(
                    "{}",
                    json!({
                        "accepted": false,
                        "partial": true,
                        "operation": "track.project.update",
                        "request_id": request_id,
                        "completed": [{"role": "new_project_head", "event_id": new_head_id}],
                        "pending": ["delete_old_project_address", "project_activity"],
                        "project_id": before.project_id,
                        "retryable": true,
                    })
                );
                return Err(error);
            }
        }
    }
    let activity_event = client.sign_event(
        build_tracker_project_activity(
            workspace_id,
            &activity,
            &client.keys().public_key().to_hex(),
        )
        .map_err(|error| usage("build project activity", error))?,
    )?;
    let (activity_id, activity_replayed) =
        match submit_idempotent(client, workspace_id, request_id, activity_event.clone()).await {
            Ok(result) => result,
            Err(error) => {
                println!(
                    "{}",
                    json!({
                        "accepted": false,
                        "partial": true,
                        "operation": "track.project.update",
                        "request_id": request_id,
                        "completed_event_ids": event_ids,
                        "pending": ["project_activity"],
                        "project_id": before.project_id,
                        "retryable": true,
                    })
                );
                return Err(error);
            }
        };
    event_ids.push(activity_id);
    all_replayed &= activity_replayed;
    if !activity_replayed {
        context.add_event(activity_event, &authorization);
        context.persist()?;
    }
    println!(
        "{}",
        json!({
            "accepted": true,
            "operation": "track.project.update",
            "request_id": request_id,
            "event_ids": event_ids,
            "replayed": all_replayed,
            "project": project_json(&next_project),
            "warnings": context.warnings,
        })
    );
    Ok(())
}

async fn cmd_project_list(
    client: &BuzzClient,
    workspace: Option<String>,
    query: Option<String>,
    limit: u32,
    cursor: Option<String>,
) -> Result<(), CliError> {
    let workspace_id = parse_workspace(workspace)?;
    let (context, _) = load_context(client, workspace_id).await?;
    let query = query.map(|value| value.to_lowercase());
    let mut projects: Vec<_> = context
        .projects
        .projects
        .values()
        .filter(|project| {
            query.as_ref().is_none_or(|needle| {
                project.name.to_lowercase().contains(needle)
                    || project.slug.to_lowercase().contains(needle)
            })
        })
        .collect();
    projects.sort_by(|left, right| {
        (left.slug.as_str(), left.project_id).cmp(&(right.slug.as_str(), right.project_id))
    });
    if let Some(cursor) = cursor {
        let cursor: Uuid = decode_cursor(&cursor)?;
        if let Some(position) = projects
            .iter()
            .position(|project| project.project_id == cursor)
        {
            projects.drain(..=position);
        }
    }
    let has_more = projects.len() > limit as usize;
    projects.truncate(limit as usize);
    let next_cursor = if has_more {
        projects
            .last()
            .map(|project| encode_cursor(&project.project_id))
            .transpose()?
    } else {
        None
    };
    println!(
        "{}",
        json!({
            "items": projects.into_iter().map(project_json).collect::<Vec<_>>(),
            "next_cursor": next_cursor,
            "warnings": context.warnings,
        })
    );
    Ok(())
}

fn parse_project_coordinate(value: &str) -> Result<Option<(&str, &str)>, CliError> {
    if !value.starts_with("30621:") {
        return Ok(None);
    }
    let mut parts = value.splitn(3, ':');
    let kind = parts.next();
    let author = parts.next();
    let slug = parts.next();
    if kind != Some("30621")
        || author.is_none_or(|value| crate::validate::validate_hex64(value).is_err())
        || slug.is_none_or(|value| value.is_empty() || value.len() > 1024)
    {
        return Err(CliError::Usage(format!(
            "invalid NIP-MP project coordinate {value:?}"
        )));
    }
    Ok(Some((author.unwrap_or_default(), slug.unwrap_or_default())))
}

async fn cmd_unmanaged_project_get(
    client: &BuzzClient,
    coordinate: &str,
    author: &str,
    slug: &str,
) -> Result<(), CliError> {
    let mut events: Vec<Event> = client
        .query_all(json!({
            "kinds": [KIND_PROJECT],
            "authors": [author],
            "#d": [slug],
            "limit": 1,
        }))
        .await?
        .into_iter()
        .map(|value| {
            serde_json::from_value(value)
                .map_err(|error| CliError::Other(format!("invalid project event JSON: {error}")))
        })
        .collect::<Result<_, _>>()?;
    events.sort_by_key(|event| std::cmp::Reverse((event.created_at, event.id)));
    let event = events
        .into_iter()
        .next()
        .ok_or_else(|| CliError::NotFound(format!("project {coordinate:?} not found")))?;
    event
        .verify()
        .map_err(|error| CliError::Other(format!("invalid project signature: {error}")))?;
    buzz_sdk::validate_project_envelope(event.tags.as_slice(), &event.content)
        .map_err(|error| usage("invalid NIP-MP project", error))?;
    let name = event
        .tags
        .iter()
        .find(|tag| tag.as_slice().first().map(String::as_str) == Some("name"))
        .and_then(|tag| tag.as_slice().get(1))
        .cloned();
    println!(
        "{}",
        json!({
            "project": {
                "coordinate": coordinate,
                "name": name,
                "slug": slug,
                "author_pubkey": author,
                "revision_event_id": event.id.to_hex(),
                "updated_at": rfc3339(event.created_at.as_secs()),
                "managed": false,
            },
            "counts": null,
            "history": [],
            "warnings": ["cross-signer NIP-MP project is referenceable by coordinate but not Tracker-manageable until its signer adopts buzz-project-id/buzz-workspace tags"],
        })
    );
    Ok(())
}

async fn cmd_project_get(
    client: &BuzzClient,
    workspace: Option<String>,
    project_key: String,
) -> Result<(), CliError> {
    let workspace_id = parse_workspace(workspace)?;
    if let Some((author, slug)) = parse_project_coordinate(&project_key)? {
        return cmd_unmanaged_project_get(client, &project_key, author, slug).await;
    }
    let (context, _) = load_context(client, workspace_id).await?;
    let project = context.project(&project_key)?;
    let (open, closed) = context
        .fold
        .issues
        .values()
        .filter(|issue| issue.project_id == Some(project.project_id))
        .fold((0usize, 0usize), |(open, closed), issue| {
            if issue.status.is_terminal() {
                (open, closed + 1)
            } else {
                (open + 1, closed)
            }
        });
    let history: Vec<_> = context
        .cache
        .events
        .iter()
        .filter_map(|event| {
            buzz_sdk::tracker::parse_tracker_project_activity(
                event,
                Utc::now().timestamp().max(0) as u64,
            )
            .ok()
        })
        .filter(|event| event.activity.project_id == project.project_id)
        .collect();
    println!(
        "{}",
        json!({
            "project": project_json(project),
            "counts": {"open": open, "closed": closed},
            "history": history,
            "warnings": context.warnings,
        })
    );
    Ok(())
}

async fn cmd_watch(
    client: &BuzzClient,
    mut filters: TrackFilterArgs,
    event_types: Vec<String>,
    since: Option<String>,
    heartbeat: u64,
    backfill: u32,
) -> Result<(), CliError> {
    if heartbeat == 0 {
        return Err(CliError::Usage("--heartbeat must be positive".into()));
    }
    let workspace_id = parse_workspace(filters.workspace.clone())?;
    let mut cursor = filters
        .cursor
        .take()
        .map(|value| decode_cursor::<EventCursor>(&value))
        .transpose()?;
    if cursor.is_none() {
        cursor = since
            .as_deref()
            .map(parse_timestamp)
            .transpose()?
            .map(|created_at| EventCursor {
                created_at,
                event_id: String::new(),
            });
    }
    let mut first = true;
    loop {
        let (context, _) = load_context(client, workspace_id).await?;
        let allowed: HashSet<Uuid> = filter_issues(client, &context, &filters)
            .await?
            .into_iter()
            .map(|issue| issue.issue_id)
            .collect();
        let mut events: Vec<_> = context
            .parsed
            .iter()
            .filter(|event| allowed.contains(&event.issue_id))
            .filter(|event| {
                cursor.as_ref().is_none_or(|cursor| {
                    (event.created_at, event.event_id.as_str())
                        > (cursor.created_at, cursor.event_id.as_str())
                })
            })
            .filter(|event| {
                let kind = match event.payload {
                    TrackerPayload::Root(_) => "created",
                    TrackerPayload::Revision(_) => "updated",
                    TrackerPayload::Comment(_) => "commented",
                };
                event_types.is_empty() || event_types.iter().any(|value| value == kind)
            })
            .collect();
        events.sort_by(|left, right| {
            (left.created_at, left.event_id.as_str())
                .cmp(&(right.created_at, right.event_id.as_str()))
        });
        if first && backfill > 0 && events.len() > backfill as usize {
            events.drain(..events.len() - backfill as usize);
        }
        for event in events {
            let imported = match &event.payload {
                TrackerPayload::Root(root) => root.import.is_some(),
                TrackerPayload::Revision(revision) => revision.import.is_some(),
                TrackerPayload::Comment(comment) => comment.content.import.is_some(),
            };
            let event_type = match event.payload {
                TrackerPayload::Root(_) => "created",
                TrackerPayload::Revision(_) => "updated",
                TrackerPayload::Comment(_) => "commented",
            };
            let next = EventCursor {
                created_at: event.created_at,
                event_id: event.event_id.clone(),
            };
            println!(
                "{}",
                json!({
                    "type": "issue_event",
                    "cursor": encode_cursor(&next)?,
                    "event": {
                        "event_id": event.event_id,
                        "issue_id": event.issue_id,
                        "event_type": event_type,
                        "actor_pubkey": event.author_pubkey,
                        "created_at": rfc3339(event.created_at),
                        "import_provenance": imported,
                        "dispatcher_suppressed": imported,
                    },
                })
            );
            cursor = Some(next);
        }
        let heartbeat_cursor = cursor.as_ref().map(encode_cursor).transpose()?;
        println!(
            "{}",
            json!({
                "type": "heartbeat",
                "cursor": heartbeat_cursor,
                "created_at": rfc3339(Utc::now().timestamp().max(0) as u64),
            })
        );
        std::io::stdout()
            .flush()
            .map_err(|error| CliError::Other(format!("watch stdout flush failed: {error}")))?;
        first = false;
        tokio::time::sleep(std::time::Duration::from_secs(heartbeat)).await;
    }
}

/// Dispatch one `buzz track` subcommand.
pub async fn dispatch(
    command: TrackCmd,
    client: &BuzzClient,
    format: &OutputFormat,
) -> Result<(), CliError> {
    match command {
        TrackCmd::Create {
            workspace,
            issue_id,
            identifier,
            channel,
            title,
            body,
            status,
            priority,
            assignee,
            project,
            label,
            due,
            tz,
            estimate,
            origin_message,
            external_id,
            mutation,
        } => {
            cmd_create(
                client,
                workspace,
                issue_id,
                identifier,
                channel,
                title,
                body,
                status,
                priority,
                assignee,
                project,
                label,
                due,
                tz,
                estimate,
                origin_message,
                external_id,
                mutation,
                format,
            )
            .await
        }
        TrackCmd::Update {
            workspace,
            issue,
            title,
            body,
            status,
            priority,
            assignee,
            project,
            due,
            tz,
            estimate,
            add_label,
            remove_label,
            channel,
            mutation,
        } => {
            cmd_update(
                client,
                workspace,
                issue,
                title,
                body,
                status,
                priority,
                assignee,
                project,
                due,
                tz,
                estimate,
                add_label,
                remove_label,
                channel,
                mutation,
                format,
            )
            .await
        }
        TrackCmd::List { filters } => cmd_list(client, filters, format).await,
        TrackCmd::Get {
            workspace,
            issue,
            no_history,
            include_comments,
            include_linked_tasks,
        } => {
            cmd_get(
                client,
                workspace,
                issue,
                no_history,
                include_comments,
                include_linked_tasks,
                format,
            )
            .await
        }
        TrackCmd::Board {
            filters,
            group_by,
            columns,
            include_empty,
            count_only,
        } => {
            cmd_board(
                client,
                filters,
                group_by,
                columns,
                include_empty,
                count_only,
                format,
            )
            .await
        }
        TrackCmd::Comment {
            workspace,
            issue,
            content,
            reply_to,
            mention,
            request_id,
            import_json,
        } => {
            cmd_comment(
                client,
                workspace,
                issue,
                content,
                reply_to,
                mention,
                request_id,
                import_json,
            )
            .await
        }
        TrackCmd::Relate {
            workspace,
            issue,
            relation_type,
            target,
            remove,
            mutation,
        } => {
            cmd_relate(
                client,
                workspace,
                issue,
                relation_type,
                target,
                remove,
                mutation,
            )
            .await
        }
        TrackCmd::Link {
            workspace,
            issue,
            link_type,
            reference,
            title,
            remove,
            mutation,
        } => {
            cmd_link(
                client, workspace, issue, link_type, reference, title, remove, mutation,
            )
            .await
        }
        TrackCmd::Dispatch {
            workspace,
            issue,
            assignee,
            channel,
            title,
            deliverables,
            verification,
            content,
            replace_active,
            set_status,
            mutation,
        } => {
            cmd_dispatch(
                client,
                workspace,
                issue,
                assignee,
                channel,
                title,
                deliverables,
                verification,
                content,
                replace_active,
                set_status,
                mutation,
            )
            .await
        }
        TrackCmd::Watch {
            filters,
            event_type,
            since,
            heartbeat,
            backfill,
        } => cmd_watch(client, filters, event_type, since, heartbeat, backfill).await,
        TrackCmd::Project(command) => match command {
            TrackProjectCmd::Create {
                workspace,
                project_id,
                name,
                slug,
                status,
                default_channel,
                default_verify,
                mutation,
            } => {
                cmd_project_create(
                    client,
                    workspace,
                    project_id,
                    name,
                    slug,
                    status,
                    default_channel,
                    default_verify,
                    mutation,
                )
                .await
            }
            TrackProjectCmd::Update {
                workspace,
                project,
                name,
                slug,
                status,
                default_channel,
                default_verify,
                map_status,
                mutation,
            } => {
                cmd_project_update(
                    client,
                    workspace,
                    project,
                    name,
                    slug,
                    status,
                    default_channel,
                    default_verify,
                    map_status,
                    mutation,
                )
                .await
            }
            TrackProjectCmd::List {
                workspace,
                query,
                limit,
                cursor,
            } => cmd_project_list(client, workspace, query, limit, cursor).await,
            TrackProjectCmd::Get { workspace, project } => {
                cmd_project_get(client, workspace, project).await
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind, Timestamp};
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn estimates_are_duration_strings_not_points() {
        assert_eq!(parse_estimate("90m").unwrap(), 90);
        assert_eq!(parse_estimate("2h").unwrap(), 120);
        assert_eq!(parse_estimate("1d").unwrap(), 480);
        assert!(parse_estimate("2").is_err());
        assert!(parse_estimate("0m").is_err());
        assert!(parse_estimate("9é").is_err());
    }

    #[test]
    fn cache_merge_keeps_same_second_events_by_id() {
        let keys = Keys::generate();
        let build = |content: &str| {
            EventBuilder::new(Kind::Custom(1), content)
                .custom_created_at(Timestamp::from(100))
                .sign_with_keys(&keys)
                .unwrap()
        };
        let first = build("first");
        let second = build("second");
        let mut events = vec![first.clone()];
        merge_events(&mut events, [second.clone(), first.clone()]);
        assert_eq!(events.len(), 2);
        let cursor = max_cursor(&events).unwrap();
        assert_eq!(cursor.created_at, 100);
        assert_eq!(cursor.event_id, first.id.to_hex().max(second.id.to_hex()));
    }

    #[test]
    fn cache_write_is_atomic_and_round_trips() {
        let temp = TempDir::new().unwrap();
        let workspace = Uuid::new_v4();
        let path = temp.path().join("tracker.json");
        let cache = CachedProjection {
            workspace_id: Some(workspace),
            cursor: Some(EventCursor {
                created_at: 10,
                event_id: "a".repeat(64),
            }),
            events: vec![],
        };
        write_cache(&path, &cache).unwrap();
        let loaded = read_cache(&path, workspace).unwrap();
        assert_eq!(loaded.cursor, cache.cursor);
    }

    #[test]
    fn project_status_maps_are_explicit() {
        let maps = parse_status_maps(&["backlog=todo".into()]).unwrap();
        assert_eq!(maps[&IssueStatus::Backlog], IssueStatus::Todo);
        assert!(parse_status_maps(&["backlog".into()]).is_err());
    }

    #[test]
    fn project_coordinate_parser_requires_full_nip_mp_address() {
        let author = "1".repeat(64);
        let coordinate = format!("30621:{author}:dispatch");
        assert_eq!(
            parse_project_coordinate(&coordinate).unwrap(),
            Some((author.as_str(), "dispatch"))
        );
        assert_eq!(parse_project_coordinate("dispatch").unwrap(), None);
        assert!(parse_project_coordinate("30621:short:dispatch").is_err());
        assert!(
            parse_project_coordinate(&format!("30622:{author}:dispatch"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn derived_project_request_ids_are_stable_and_scoped() {
        let request = Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap();
        let first = derived_request_id(request, "status:one");
        assert_eq!(first, derived_request_id(request, "status:one"));
        assert_ne!(first, derived_request_id(request, "status:two"));
        assert_eq!(first.get_version_num(), 5);
    }

    #[test]
    fn generated_identifiers_are_request_stable_and_collision_safe() {
        let first = Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap();
        let second = Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap();
        assert_eq!(
            allocate_identifier("SC", first),
            "SC-11111111111141118111111111111111"
        );
        assert_ne!(
            allocate_identifier("SC", first),
            allocate_identifier("SC", second)
        );
    }

    #[test]
    fn active_task_summary_excludes_terminal_tasks() {
        let issue_id = Uuid::new_v4();
        let active_id = Uuid::new_v4();
        let done_id = Uuid::new_v4();
        let make = |task_id, status, created_at| TaskEnvelopeEvent {
            event_id: format!("{created_at:064x}"),
            author_pubkey: "cc".repeat(32),
            channel_id: Uuid::nil(),
            created_at,
            envelope: TaskEnvelope {
                v: TASK_ENVELOPE_VERSION,
                task_id,
                status,
                assignee: "dd".repeat(32),
                assigner: "ee".repeat(32),
                title: "test".into(),
                deliverables: vec![],
                verification: TaskVerification {
                    method: "test".into(),
                    evidence_ref: None,
                },
                blocked_on: None,
                notes: None,
                tracker_issue_id: Some(issue_id),
                dispatch_request_id: None,
            },
        };
        let tasks = BTreeMap::from([
            (active_id, make(active_id, TaskStatus::PickedUp, 1)),
            (done_id, make(done_id, TaskStatus::Done, 2)),
        ]);
        assert_eq!(active_task_ids(&tasks).get(&issue_id), Some(&active_id));
    }

    #[test]
    fn request_semantics_ignore_revision_chain_metadata_only() {
        let keys = Keys::generate();
        let build = |parent: &str, revision: u64, title: &str| {
            EventBuilder::new(
                Kind::Custom(KIND_TRACKER_REVISION as u16),
                json!({
                    "v": 1,
                    "op": "revise",
                    "issue_id": "11111111-1111-4111-8111-111111111111",
                    "root_event": "a".repeat(64),
                    "parent_event": parent,
                    "revision": revision,
                    "set": {"title": title},
                    "clear": [],
                    "relations": {"add": [], "remove": []},
                    "external_refs": {"add": [], "remove": []},
                    "reason": null,
                    "request_id": "22222222-2222-4222-8222-222222222222",
                    "import": null,
                })
                .to_string(),
            )
            .tag(Tag::parse(["h", "33333333-3333-4333-8333-333333333333"]).unwrap())
            .tag(Tag::parse(["d", "11111111-1111-4111-8111-111111111111"]).unwrap())
            .tag(Tag::parse(["t", "buzz-tracker"]).unwrap())
            .tag(Tag::parse(["e", parent, "", "reply"]).unwrap())
            .tag(Tag::parse(["request", "22222222-2222-4222-8222-222222222222"]).unwrap())
            .sign_with_keys(&keys)
            .unwrap()
        };
        let first = build(&"a".repeat(64), 1, "same intent");
        let retry = build(&"b".repeat(64), 2, "same intent");
        let conflict = build(&"b".repeat(64), 2, "different intent");
        assert!(same_semantic_request(&first, &retry));
        assert!(!same_semantic_request(&first, &conflict));
    }
}
