//! Versioned task-envelope metadata carried by ordinary kind-9 messages.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use nostr::{Event, EventBuilder, Kind, Tag};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{SdkError, ThreadRef};

/// Current task-envelope schema version.
pub const TASK_ENVELOPE_VERSION: u8 = 3;
/// Standard NIP-23 hashtag used to discover task-envelope messages.
pub const TASK_DISCOVERY_HASHTAG: &str = "buzz-task";

/// Supported task lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Work has been assigned but not yet claimed.
    Assigned,
    /// The assignee has started work.
    PickedUp,
    /// Work cannot progress without an external dependency.
    Blocked,
    /// The assignee reports the work complete.
    Done,
    /// The attempt ended unsuccessfully.
    Failed,
    /// An assigner or delegated reviewer independently checked the result.
    Verified,
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Assigned => "assigned",
            Self::PickedUp => "picked_up",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Verified => "verified",
        })
    }
}

impl FromStr for TaskStatus {
    type Err = SdkError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "assigned" => Ok(Self::Assigned),
            "picked_up" => Ok(Self::PickedUp),
            "blocked" => Ok(Self::Blocked),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            "verified" => Ok(Self::Verified),
            _ => Err(SdkError::InvalidInput(format!(
                "unknown task status {value:?}"
            ))),
        }
    }
}

/// Supported task deliverable reference types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskDeliverableType {
    /// A Buzz or external pull request.
    Pr,
    /// A git commit.
    Commit,
    /// A file path.
    File,
    /// A document reference.
    Doc,
    /// A Nostr/Buzz event id.
    Event,
    /// A URL.
    Url,
}

impl fmt::Display for TaskDeliverableType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pr => "pr",
            Self::Commit => "commit",
            Self::File => "file",
            Self::Doc => "doc",
            Self::Event => "event",
            Self::Url => "url",
        })
    }
}

impl FromStr for TaskDeliverableType {
    type Err = SdkError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pr" => Ok(Self::Pr),
            "commit" => Ok(Self::Commit),
            "file" => Ok(Self::File),
            "doc" => Ok(Self::Doc),
            "event" => Ok(Self::Event),
            "url" => Ok(Self::Url),
            _ => Err(SdkError::InvalidInput(format!(
                "unknown task deliverable type {value:?}"
            ))),
        }
    }
}

/// A typed reference to a task deliverable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDeliverable {
    /// Reference type.
    #[serde(rename = "type")]
    pub kind: TaskDeliverableType,
    /// Commit, event, file, document, PR, or URL reference.
    #[serde(rename = "ref")]
    pub reference: String,
}

/// Requested verification and optional evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskVerification {
    /// Human-readable verification method.
    pub method: String,
    /// Evidence reference supplied at completion or verification time.
    pub evidence_ref: Option<String>,
}

/// Authority fixed by the signed assignment, independently of later status claims.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskAuthority {
    /// Assignment event ID. Absent on the assignment itself; required on v3 updates.
    pub assignment_event_id: Option<String>,
    /// Accepted predecessor. Establishes causality even within one timestamp second.
    pub previous_event_id: Option<String>,
    /// Optional independent reviewer delegated by the assigner at assignment time.
    pub reviewer: Option<String>,
}

/// Versioned task metadata appended to a human-readable channel message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskEnvelope {
    /// Schema version. Must equal [`TASK_ENVELOPE_VERSION`].
    pub v: u8,
    /// Stable task UUID.
    pub task_id: Uuid,
    /// Current lifecycle state claimed by this event.
    pub status: TaskStatus,
    /// Assigned agent pubkey as 64-character lowercase hex.
    pub assignee: String,
    /// Original assigner pubkey as 64-character lowercase hex.
    pub assigner: String,
    /// Short human-readable task title.
    pub title: String,
    /// Deliverables known at this transition.
    pub deliverables: Vec<TaskDeliverable>,
    /// Verification contract and optional evidence.
    pub verification: TaskVerification,
    /// Current blocking dependency, if any.
    pub blocked_on: Option<String>,
    /// Optional lifecycle note.
    pub notes: Option<String>,
    /// Tracker issue correlated with this execution task, when any.
    #[serde(default)]
    pub tracker_issue_id: Option<Uuid>,
    /// Idempotency key assigned by the dispatcher for this execution request.
    #[serde(default)]
    pub dispatch_request_id: Option<Uuid>,
    /// Version 3 assignment anchor and delegated reviewer; absent in historical v1/v2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<TaskAuthority>,
}

impl TaskEnvelope {
    /// Validate and normalize an envelope before build or after parse.
    pub fn validate(mut self) -> Result<Self, SdkError> {
        if !matches!(self.v, 1 | 2 | TASK_ENVELOPE_VERSION) {
            return Err(SdkError::InvalidInput(format!(
                "task envelope v must be 1, 2 or {TASK_ENVELOPE_VERSION}"
            )));
        }
        if self.v == 1 && (self.tracker_issue_id.is_some() || self.dispatch_request_id.is_some()) {
            return Err(SdkError::InvalidInput(
                "task envelope v1 cannot carry Tracker correlation fields".into(),
            ));
        }
        self.assignee = normalize_pubkey(&self.assignee, "assignee")?;
        self.assigner = normalize_pubkey(&self.assigner, "assigner")?;
        if self.v < 3 && self.authority.is_some() {
            return Err(SdkError::InvalidInput(
                "task authority requires envelope v3".into(),
            ));
        }
        if self.v >= 3 {
            let authority = self.authority.as_mut().ok_or_else(|| {
                SdkError::InvalidInput("task v3 requires assignment authority".into())
            })?;
            authority.reviewer = authority
                .reviewer
                .as_deref()
                .map(|value| normalize_pubkey(value, "authority.reviewer"))
                .transpose()?;
            if authority.reviewer.as_deref() == Some(self.assignee.as_str()) {
                return Err(SdkError::InvalidInput(
                    "the executor cannot be the independent reviewer".into(),
                ));
            }
            authority.assignment_event_id = authority
                .assignment_event_id
                .as_deref()
                .map(|value| normalize_pubkey(value, "authority.assignment_event_id"))
                .transpose()?;
            authority.previous_event_id = authority
                .previous_event_id
                .as_deref()
                .map(|value| normalize_pubkey(value, "authority.previous_event_id"))
                .transpose()?;
            if (self.status == TaskStatus::Assigned) != authority.assignment_event_id.is_none() {
                return Err(SdkError::InvalidInput("task updates must reference the signed assignment; assignments cannot reference another assignment".into()));
            }
            if (self.status == TaskStatus::Assigned) != authority.previous_event_id.is_none() {
                return Err(SdkError::InvalidInput(
                    "task updates must reference their accepted predecessor".into(),
                ));
            }
        }
        self.title = bounded_nonempty(self.title, "title", 256)?;
        self.verification.method =
            bounded_nonempty(self.verification.method, "verification.method", 2048)?;
        self.verification.evidence_ref = normalize_optional(
            self.verification.evidence_ref,
            "verification.evidence_ref",
            4096,
        )?;
        self.blocked_on = normalize_optional(self.blocked_on, "blocked_on", 4096)?;
        self.notes = normalize_optional(self.notes, "notes", 4096)?;
        for deliverable in &mut self.deliverables {
            deliverable.reference = bounded_nonempty(
                std::mem::take(&mut deliverable.reference),
                "deliverables[].ref",
                4096,
            )?;
        }
        Ok(self)
    }
}

/// Parsed event metadata used for history and deterministic folding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskEnvelopeEvent {
    /// Nostr event id.
    pub event_id: String,
    /// Nostr event author pubkey.
    pub author_pubkey: String,
    /// Channel UUID from the `h` tag.
    pub channel_id: Uuid,
    /// Nostr event timestamp in seconds.
    pub created_at: u64,
    /// Validated task envelope.
    pub envelope: TaskEnvelope,
}

impl TaskEnvelopeEvent {
    /// Parse and cross-check a signed kind-9 event.
    pub fn from_event(event: &Event) -> Result<Self, SdkError> {
        event
            .verify()
            .map_err(|error| SdkError::InvalidInput(format!("invalid task signature: {error}")))?;
        if event.kind != Kind::Custom(9) {
            return Err(SdkError::InvalidInput(
                "task envelope event must be kind 9".into(),
            ));
        }
        let envelope = parse_task_envelope(&event.content)?;
        let mut channel_tags = Vec::new();
        let mut task_tags = Vec::new();
        let mut discovery = false;
        for tag in event.tags.iter() {
            let values = tag.as_slice();
            match values.first().map(String::as_str) {
                Some("h") => {
                    channel_tags.push(values);
                }
                Some("task") => {
                    task_tags.push(values);
                }
                Some("t") if values.get(1).map(String::as_str) == Some(TASK_DISCOVERY_HASHTAG) => {
                    discovery = true;
                }
                _ => {}
            }
        }
        if channel_tags.len() != 1 || channel_tags[0].len() != 2 {
            return Err(SdkError::InvalidInput(
                "task envelope event requires exactly one well-formed h tag".into(),
            ));
        }
        let channel_id = Uuid::parse_str(&channel_tags[0][1]).map_err(|_| {
            SdkError::InvalidInput("task envelope event requires a valid h tag".into())
        })?;
        if task_tags.len() != 1 || task_tags[0].len() != 3 {
            return Err(SdkError::InvalidInput(
                "task envelope event requires exactly one well-formed task tag".into(),
            ));
        }
        let expected_task_id = envelope.task_id.to_string();
        let expected_status = envelope.status.to_string();
        if (task_tags[0][1].as_str(), task_tags[0][2].as_str())
            != (expected_task_id.as_str(), expected_status.as_str())
        {
            return Err(SdkError::InvalidInput(
                "task tag does not match envelope task_id/status".into(),
            ));
        }
        if !discovery {
            return Err(SdkError::InvalidInput(
                "task envelope event requires t:buzz-task discovery tag".into(),
            ));
        }
        Ok(Self {
            event_id: event.id.to_hex(),
            author_pubkey: event.pubkey.to_hex(),
            channel_id,
            created_at: event.created_at.as_secs(),
            envelope,
        })
    }
}

/// Append a fenced `buzz-task` JSON block to ordinary human-readable content.
pub fn append_task_envelope(
    human_content: &str,
    envelope: &TaskEnvelope,
) -> Result<String, SdkError> {
    let envelope = envelope.clone().validate()?;
    if envelope.v != TASK_ENVELOPE_VERSION {
        return Err(SdkError::InvalidInput(format!(
            "new task messages must use task envelope v{TASK_ENVELOPE_VERSION}"
        )));
    }
    let json = serde_json::to_string_pretty(&envelope).map_err(|error| {
        SdkError::InvalidInput(format!("task envelope serialization failed: {error}"))
    })?;
    let content = human_content.trim_end();
    let separator = if content.is_empty() { "" } else { "\n\n" };
    let result = format!("{content}{separator}```buzz-task\n{json}\n```");
    if result.len() > 64 * 1024 {
        return Err(SdkError::ContentTooLarge {
            max: 64 * 1024,
            got: result.len(),
        });
    }
    Ok(result)
}

/// Parse a task envelope only when it is the final fenced block in content.
pub fn parse_task_envelope(content: &str) -> Result<TaskEnvelope, SdkError> {
    let trimmed = content.trim_end_matches([' ', '\t', '\r', '\n']);
    let prefix = "```buzz-task\n";
    let start = trimmed
        .rfind(prefix)
        .ok_or_else(|| SdkError::InvalidInput("message has no trailing buzz-task block".into()))?;
    if start > 0 && !trimmed[..start].ends_with('\n') {
        return Err(SdkError::InvalidInput(
            "buzz-task fence must start on its own line".into(),
        ));
    }
    let fenced = &trimmed[start + prefix.len()..];
    let json = fenced
        .strip_suffix("\n```")
        .ok_or_else(|| SdkError::InvalidInput("buzz-task block must end the message".into()))?;
    if json.contains("\n```") {
        return Err(SdkError::InvalidInput(
            "buzz-task block contains an early closing fence".into(),
        ));
    }
    serde_json::from_str::<TaskEnvelope>(json)
        .map_err(|error| SdkError::InvalidInput(format!("malformed buzz-task JSON: {error}")))?
        .validate()
}

/// Build a normal kind-9 channel message with validated task metadata.
pub fn build_task_message(
    channel_id: Uuid,
    human_content: &str,
    envelope: &TaskEnvelope,
    thread_ref: Option<&ThreadRef>,
    mentions: &[&str],
) -> Result<EventBuilder, SdkError> {
    let envelope = envelope.clone().validate()?;
    if mentions.len() > crate::mentions::MENTION_CAP {
        return Err(SdkError::TooManyMentions);
    }
    let content = append_task_envelope(human_content, &envelope)?;
    let channel = channel_id.to_string();
    let task_id = envelope.task_id.to_string();
    let status = envelope.status.to_string();
    let mut tags = vec![
        parse_tag(&["h", &channel])?,
        parse_tag(&["task", &task_id, &status])?,
        parse_tag(&["t", TASK_DISCOVERY_HASHTAG])?,
    ];
    if let Some(thread) = thread_ref {
        let root = thread.root_event_id.to_hex();
        let parent = thread.parent_event_id.to_hex();
        if root == parent {
            tags.push(parse_tag(&["e", &root, "", "reply"])?);
        } else {
            tags.push(parse_tag(&["e", &root, "", "root"])?);
            tags.push(parse_tag(&["e", &parent, "", "reply"])?);
        }
    }
    let mut seen = std::collections::HashSet::new();
    for mention in mentions {
        let mention = normalize_pubkey(mention, "mention")?;
        if seen.insert(mention.clone()) {
            tags.push(parse_tag(&["p", &mention])?);
        }
    }
    Ok(EventBuilder::new(Kind::Custom(9), content).tags(tags))
}

/// Fold authorized task history in deterministic order. Rejected claims never advance state.
pub fn fold_task_events(
    events: impl IntoIterator<Item = TaskEnvelopeEvent>,
) -> BTreeMap<Uuid, TaskEnvelopeEvent> {
    crate::task_authority::fold_authorized_task_events(events).current
}

fn parse_tag(parts: &[&str]) -> Result<Tag, SdkError> {
    Tag::parse(parts.iter().copied()).map_err(|error| SdkError::InvalidTag(error.to_string()))
}

fn normalize_pubkey(value: &str, field: &str) -> Result<String, SdkError> {
    if value.len() != 64 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(SdkError::InvalidInput(format!(
            "{field} must be a 64-character hex pubkey"
        )));
    }
    Ok(value.to_ascii_lowercase())
}

fn bounded_nonempty(value: String, field: &str, max: usize) -> Result<String, SdkError> {
    let value = value.trim().to_string();
    if value.is_empty() || value.len() > max {
        return Err(SdkError::InvalidInput(format!(
            "{field} must contain 1..={max} bytes"
        )));
    }
    Ok(value)
}

fn normalize_optional(
    value: Option<String>,
    field: &str,
    max: usize,
) -> Result<Option<String>, SdkError> {
    value
        .map(|value| bounded_nonempty(value, field, max))
        .transpose()
}

#[cfg(test)]
mod tests {
    use nostr::Keys;
    use serde::Deserialize;

    use super::*;

    fn envelope(status: TaskStatus) -> TaskEnvelope {
        TaskEnvelope {
            v: TASK_ENVELOPE_VERSION,
            authority: Some(TaskAuthority {
                assignment_event_id: (status != TaskStatus::Assigned).then(|| "11".repeat(32)),
                previous_event_id: (status != TaskStatus::Assigned).then(|| "11".repeat(32)),
                reviewer: None,
            }),
            task_id: Uuid::parse_str("11111111-2222-4333-8444-555555555555").unwrap(),
            status,
            assignee: "aa".repeat(32),
            assigner: "bb".repeat(32),
            title: "Implement task envelopes".into(),
            deliverables: vec![TaskDeliverable {
                kind: TaskDeliverableType::Pr,
                reference: "event-id".into(),
            }],
            verification: TaskVerification {
                method: "full suite".into(),
                evidence_ref: None,
            },
            blocked_on: None,
            notes: None,
            tracker_issue_id: None,
            dispatch_request_id: None,
        }
    }

    #[test]
    fn build_and_parse_round_trip_with_matching_tags() {
        let keys = Keys::generate();
        let channel = Uuid::new_v4();
        let event = build_task_message(
            channel,
            "Please implement this.",
            &envelope(TaskStatus::Assigned),
            None,
            &[&keys.public_key().to_hex()],
        )
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
        let parsed = TaskEnvelopeEvent::from_event(&event).unwrap();
        assert_eq!(parsed.channel_id, channel);
        assert_eq!(parsed.envelope.status, TaskStatus::Assigned);
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["t", TASK_DISCOVERY_HASHTAG]));
    }

    #[test]
    fn malformed_or_non_trailing_envelopes_are_rejected() {
        assert!(parse_task_envelope("```buzz-task\nnot-json\n```").is_err());
        let content = append_task_envelope("human", &envelope(TaskStatus::Done)).unwrap();
        assert!(parse_task_envelope(&format!("{content}\ntrailing text")).is_err());
        assert!(parse_task_envelope(&content.replace("\"v\": 3", "\"v\": 4")).is_err());
    }

    #[test]
    fn mismatched_task_tag_is_rejected() {
        let keys = Keys::generate();
        let channel = Uuid::new_v4();
        let content = append_task_envelope("done", &envelope(TaskStatus::Done)).unwrap();
        let event = EventBuilder::new(Kind::Custom(9), content)
            .tags([
                parse_tag(&["h", &channel.to_string()]).unwrap(),
                parse_tag(&["task", "11111111-2222-4333-8444-555555555555", "failed"]).unwrap(),
                parse_tag(&["t", TASK_DISCOVERY_HASHTAG]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        assert!(TaskEnvelopeEvent::from_event(&event).is_err());
    }

    #[test]
    fn duplicate_or_unknown_envelope_fields_are_rejected() {
        let content = append_task_envelope("human", &envelope(TaskStatus::Done)).unwrap();
        assert!(parse_task_envelope(&content.replace(
            "\"notes\": null",
            "\"notes\": null,\n  \"priority\": \"high\""
        ))
        .is_err());

        let keys = Keys::generate();
        let channel = Uuid::new_v4();
        let task = envelope(TaskStatus::Done);
        let content = append_task_envelope("done", &task).unwrap();
        let event = EventBuilder::new(Kind::Custom(9), content)
            .tags([
                parse_tag(&["h", &channel.to_string()]).unwrap(),
                parse_tag(&["task", &task.task_id.to_string(), "done"]).unwrap(),
                parse_tag(&["task", &task.task_id.to_string(), "done"]).unwrap(),
                parse_tag(&["t", TASK_DISCOVERY_HASHTAG]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        assert!(TaskEnvelopeEvent::from_event(&event).is_err());
    }

    #[test]
    fn fold_uses_created_at_then_event_id() {
        let task_id = envelope(TaskStatus::Assigned).task_id;
        let mk = |created_at, event_id: &str, status| TaskEnvelopeEvent {
            event_id: event_id.into(),
            author_pubkey: if status == TaskStatus::Done {
                "aa"
            } else {
                "bb"
            }
            .repeat(32),
            channel_id: Uuid::nil(),
            created_at,
            envelope: TaskEnvelope {
                v: 2,
                authority: None,
                verification: TaskVerification {
                    method: "full suite".into(),
                    evidence_ref: Some("review receipt".into()),
                },
                ..envelope(status)
            },
        };
        let current = fold_task_events([
            mk(11, "a", TaskStatus::Done),
            mk(10, "z", TaskStatus::Assigned),
            mk(11, "b", TaskStatus::Verified),
        ]);
        assert_eq!(current[&task_id].envelope.status, TaskStatus::Verified);
    }

    #[test]
    fn v3_tracker_correlation_round_trips() {
        let tracker_issue_id = Uuid::new_v4();
        let dispatch_request_id = Uuid::new_v4();
        let mut task = envelope(TaskStatus::Assigned);
        task.tracker_issue_id = Some(tracker_issue_id);
        task.dispatch_request_id = Some(dispatch_request_id);
        let content = append_task_envelope("dispatch", &task).unwrap();
        let parsed = parse_task_envelope(&content).unwrap();
        assert_eq!(parsed.v, TASK_ENVELOPE_VERSION);
        assert_eq!(parsed.tracker_issue_id, Some(tracker_issue_id));
        assert_eq!(parsed.dispatch_request_id, Some(dispatch_request_id));
    }

    #[test]
    fn v3_reader_preserves_historical_v1_envelopes() {
        let mut historical = serde_json::to_value(envelope(TaskStatus::Assigned)).unwrap();
        historical["v"] = serde_json::json!(1);
        let fields = historical.as_object_mut().unwrap();
        fields.remove("authority");
        fields.remove("tracker_issue_id");
        fields.remove("dispatch_request_id");
        let legacy = format!("legacy\n\n```buzz-task\n{}\n```", historical);
        let parsed = parse_task_envelope(&legacy).unwrap();
        assert_eq!(parsed.v, 1);
        assert_eq!(parsed.tracker_issue_id, None);
        assert_eq!(parsed.dispatch_request_id, None);
    }

    #[test]
    fn v1_deny_unknown_reader_fails_loud_on_v3() {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(dead_code)]
        struct LegacyEnvelope {
            v: u8,
            task_id: Uuid,
            status: TaskStatus,
            assignee: String,
            assigner: String,
            title: String,
            deliverables: Vec<TaskDeliverable>,
            verification: TaskVerification,
            blocked_on: Option<String>,
            notes: Option<String>,
        }

        let content = append_task_envelope("dispatch", &envelope(TaskStatus::Assigned)).unwrap();
        let json = content
            .split_once("```buzz-task\n")
            .expect("task fence")
            .1
            .strip_suffix("\n```")
            .expect("closing fence");
        assert!(serde_json::from_str::<LegacyEnvelope>(json).is_err());
    }
}
