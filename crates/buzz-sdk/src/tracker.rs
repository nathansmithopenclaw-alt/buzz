//! Strict NIP-BT Tracker parsing and deterministic state folding.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use buzz_core::kind::{
    KIND_PROJECT, KIND_TRACKER_COMMENT, KIND_TRACKER_ISSUE, KIND_TRACKER_PROJECT_ACTIVITY,
    KIND_TRACKER_REVISION,
};
use chrono::{TimeZone, Timelike};
use chrono_tz::Tz;
use nostr::{Event, EventBuilder, Kind, Tag};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::SdkError;

/// Current Tracker wire schema version.
pub const TRACKER_VERSION: u8 = 1;
/// Discovery hashtag carried by every Tracker event.
pub const TRACKER_DISCOVERY_TAG: &str = "buzz-tracker";
/// Maximum accepted event clock skew into the future.
pub const MAX_FUTURE_SKEW_SECONDS: u64 = 300;

/// Stable Tracker workflow states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    /// Awaiting triage.
    Triage,
    /// Accepted but not yet scheduled.
    Backlog,
    /// Scheduled for execution.
    Todo,
    /// Actively being executed.
    InProgress,
    /// Awaiting independent review.
    InReview,
    /// Completed terminal state.
    Done,
    /// Cancelled terminal state.
    Cancelled,
}

impl IssueStatus {
    /// Whether this status is terminal.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Cancelled)
    }
}

impl std::fmt::Display for IssueStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(status_str(*self))
    }
}

impl FromStr for IssueStatus {
    type Err = SdkError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "triage" => Ok(Self::Triage),
            "backlog" => Ok(Self::Backlog),
            "todo" => Ok(Self::Todo),
            "in_progress" => Ok(Self::InProgress),
            "in_review" => Ok(Self::InReview),
            "done" => Ok(Self::Done),
            "cancelled" => Ok(Self::Cancelled),
            _ => invalid(
                "status must be triage, backlog, todo, in_progress, in_review, done, or cancelled",
            ),
        }
    }
}

/// Issue priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Priority {
    /// Immediate owner decision or incident response.
    P0,
    /// High priority.
    P1,
    /// Normal priority.
    P2,
    /// Low priority.
    P3,
}

impl std::fmt::Display for Priority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(priority_str(*self))
    }
}

impl FromStr for Priority {
    type Err = SdkError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "P0" | "p0" => Ok(Self::P0),
            "P1" | "p1" => Ok(Self::P1),
            "P2" | "p2" => Ok(Self::P2),
            "P3" | "p3" => Ok(Self::P3),
            _ => invalid("priority must be P0, P1, P2, or P3"),
        }
    }
}

/// Due-date precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuePrecision {
    /// Exact instant.
    Instant,
    /// Local calendar date.
    Date,
}

/// Due-date value with calendar semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Due {
    /// Unix timestamp in seconds.
    pub value: u64,
    /// Whether the value is an instant or date.
    pub precision: DuePrecision,
    /// IANA time-zone identifier.
    pub timezone: String,
}

/// Typed import provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportProvenance {
    /// Source system identifier.
    pub system: String,
    /// Stable identifier in the source system.
    pub external_id: String,
    /// Optional source URL.
    pub source_url: Option<String>,
    /// Import timestamp.
    pub imported_at: u64,
    /// Original source creation timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_created_at: Option<String>,
    /// Original source update timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_updated_at: Option<String>,
    /// Original source completion timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_completed_at: Option<String>,
    /// Original source cancellation timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_cancelled_at: Option<String>,
    /// Whether the source record was archived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    /// Source-system estimate points (never converted to minutes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate_points: Option<u32>,
    /// Complete export SHA-256 used for this import.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export_sha256: Option<String>,
    /// Importer implementation version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub importer_version: Option<String>,
    /// Original human author identity as source data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_author: Option<String>,
    /// Legacy labels retained during pilot reconciliation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub legacy_labels: Vec<String>,
}

/// External reference identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalRef {
    /// Reference namespace, such as `nip34_issue` or `pull_request`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Namespace-local stable reference.
    #[serde(rename = "ref")]
    pub reference: String,
}

/// Canonical directed issue relation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relation {
    /// Canonical relation type.
    #[serde(rename = "type")]
    pub kind: RelationType,
    /// Target issue UUID.
    pub target_issue_id: Uuid,
    /// Target immutable root event ID.
    pub target_root_event: String,
}

/// Canonical relation types stored on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationType {
    /// Source blocks target.
    Blocks,
    /// Source duplicates target.
    DuplicateOf,
    /// Source is parent of target.
    Parent,
}

/// User-facing relation direction accepted by the builder canonicalizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationDirection {
    /// Source blocks target.
    Blocks,
    /// Source is blocked by target.
    BlockedBy,
    /// Source duplicates target.
    DuplicateOf,
    /// Source is parent of target.
    Parent,
    /// Source is child of target.
    Child,
}

/// Canonicalized relation write, including the issue on which it must be stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalRelationWrite {
    /// Issue UUID on which the canonical relation is stored.
    pub source_issue_id: Uuid,
    /// Source root event ID.
    pub source_root_event: String,
    /// Canonical relation payload.
    pub relation: Relation,
}

/// Canonicalize inverse relation vocabulary before publishing.
pub fn canonicalize_relation(
    source_issue_id: Uuid,
    source_root_event: String,
    direction: RelationDirection,
    target_issue_id: Uuid,
    target_root_event: String,
) -> CanonicalRelationWrite {
    match direction {
        RelationDirection::Blocks => direct_relation(
            source_issue_id,
            source_root_event,
            RelationType::Blocks,
            target_issue_id,
            target_root_event,
        ),
        RelationDirection::DuplicateOf => direct_relation(
            source_issue_id,
            source_root_event,
            RelationType::DuplicateOf,
            target_issue_id,
            target_root_event,
        ),
        RelationDirection::Parent => direct_relation(
            source_issue_id,
            source_root_event,
            RelationType::Parent,
            target_issue_id,
            target_root_event,
        ),
        RelationDirection::BlockedBy | RelationDirection::Child => CanonicalRelationWrite {
            source_issue_id: target_issue_id,
            source_root_event: target_root_event,
            relation: Relation {
                kind: if direction == RelationDirection::BlockedBy {
                    RelationType::Blocks
                } else {
                    RelationType::Parent
                },
                target_issue_id: source_issue_id,
                target_root_event: source_root_event,
            },
        },
    }
}

fn direct_relation(
    source_issue_id: Uuid,
    source_root_event: String,
    kind: RelationType,
    target_issue_id: Uuid,
    target_root_event: String,
) -> CanonicalRelationWrite {
    CanonicalRelationWrite {
        source_issue_id,
        source_root_event,
        relation: Relation {
            kind,
            target_issue_id,
            target_root_event,
        },
    }
}

/// Strict issue-root content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueRoot {
    /// Schema version.
    pub v: u8,
    /// Must be `create`.
    pub op: String,
    /// Stable issue UUID.
    pub issue_id: Uuid,
    /// Human-facing workspace identifier.
    pub identifier: String,
    /// Issue title.
    pub title: String,
    /// Markdown body.
    pub body: String,
    /// Workflow state.
    pub status: IssueStatus,
    /// Optional priority.
    pub priority: Option<Priority>,
    /// Optional accountable assignee pubkey.
    pub assignee: Option<String>,
    /// Optional project UUID.
    pub project_id: Option<Uuid>,
    /// Optional due value.
    pub due: Option<Due>,
    /// Optional estimate in minutes.
    pub estimate_minutes: Option<u32>,
    /// Optional human-visible home channel.
    pub home_channel_id: Option<Uuid>,
    /// Optional source event ID.
    pub source_event_id: Option<String>,
    /// Workflow identifier.
    pub workflow: String,
    /// Normalized labels.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Initial external references.
    #[serde(default)]
    pub external_refs: Vec<ExternalRef>,
    /// Optional idempotency request ID.
    pub request_id: Option<Uuid>,
    /// Optional import provenance.
    pub import: Option<ImportProvenance>,
}

/// Sparse set operation in a revision.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionSet {
    /// Replacement identifier.
    pub identifier: Option<String>,
    /// Replacement title.
    pub title: Option<String>,
    /// Replacement Markdown body.
    pub body: Option<String>,
    /// Replacement status.
    pub status: Option<IssueStatus>,
    /// Replacement priority.
    pub priority: Option<Priority>,
    /// Replacement assignee.
    pub assignee: Option<String>,
    /// Replacement project.
    pub project_id: Option<Uuid>,
    /// Replacement due value.
    pub due: Option<Due>,
    /// Replacement estimate in minutes.
    pub estimate_minutes: Option<u32>,
    /// Replacement home channel.
    pub home_channel_id: Option<Uuid>,
    /// Replacement workflow.
    pub workflow: Option<String>,
    /// Complete replacement labels value.
    pub labels: Option<Vec<String>>,
}

/// Add/remove operations for a set-valued field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = "T: Deserialize<'de>"))]
pub struct SetOperations<T> {
    /// Values to add.
    #[serde(default)]
    pub add: Vec<T>,
    /// Values to remove.
    #[serde(default)]
    pub remove: Vec<T>,
}

impl<T> Default for SetOperations<T> {
    fn default() -> Self {
        Self {
            add: Vec::new(),
            remove: Vec::new(),
        }
    }
}

/// Strict issue-revision content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueRevision {
    /// Schema version.
    pub v: u8,
    /// Must be `revise`.
    pub op: String,
    /// Stable issue UUID.
    pub issue_id: Uuid,
    /// Immutable root event ID.
    pub root_event: String,
    /// Parent root or revision event ID.
    pub parent_event: String,
    /// Parent-relative revision integer.
    pub revision: u64,
    /// Sparse scalar replacements.
    #[serde(default)]
    pub set: RevisionSet,
    /// Nullable fields to clear.
    #[serde(default)]
    pub clear: Vec<String>,
    /// Canonical relation operations.
    #[serde(default)]
    pub relations: SetOperations<Relation>,
    /// External-reference operations keyed by `(type, ref)`.
    #[serde(default)]
    pub external_refs: SetOperations<ExternalRef>,
    /// Reason required for every status transition.
    pub reason: Option<String>,
    /// Optional idempotency request ID.
    pub request_id: Option<Uuid>,
    /// Optional import provenance.
    pub import: Option<ImportProvenance>,
}

/// Strict Tracker comment content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueComment {
    /// Schema version.
    pub v: u8,
    /// Stable issue UUID.
    pub issue_id: Uuid,
    /// Markdown comment body.
    pub body: String,
    /// Optional request ID.
    pub request_id: Option<Uuid>,
    /// Optional import provenance.
    pub import: Option<ImportProvenance>,
}

/// Comment content plus its strict root/reply tag projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerComment {
    /// Strict comment JSON content.
    pub content: IssueComment,
    /// Immutable issue root event ID.
    pub root_event: String,
    /// Optional parent comment event ID.
    pub parent_event: Option<String>,
}

/// Current Tracker-managed NIP-MP project head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerProject {
    /// Immutable Tracker project UUID from `buzz-project-id`.
    pub project_id: Uuid,
    /// Workspace Tracker stream from `buzz-workspace`.
    pub workspace_id: Uuid,
    /// Current NIP-MP address key (`d` tag).
    pub slug: String,
    /// Human-facing name.
    pub name: String,
    /// Ordered statuses enabled for this project.
    pub statuses: Vec<IssueStatus>,
    /// Default channel for issue creation/dispatch.
    pub default_channel_id: Option<Uuid>,
    /// Default dispatch verification contract.
    pub default_verify: Option<String>,
    /// Project signer.
    pub author_pubkey: String,
    /// Current head event ID.
    pub revision_event_id: String,
    /// Current head timestamp.
    pub updated_at: u64,
}

/// Snapshot stored in append-only project activity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackerProjectSnapshot {
    /// Immutable project UUID.
    pub project_id: Uuid,
    /// Current slug.
    pub slug: String,
    /// Human-facing name.
    pub name: String,
    /// Ordered workflow statuses.
    pub statuses: Vec<IssueStatus>,
    /// Default home channel.
    pub default_channel_id: Option<Uuid>,
    /// Default dispatch verification method.
    pub default_verify: Option<String>,
    /// NIP-MP head event ID represented by this snapshot.
    pub revision_event_id: String,
}

impl From<&TrackerProject> for TrackerProjectSnapshot {
    fn from(project: &TrackerProject) -> Self {
        Self {
            project_id: project.project_id,
            slug: project.slug.clone(),
            name: project.name.clone(),
            statuses: project.statuses.clone(),
            default_channel_id: project.default_channel_id,
            default_verify: project.default_verify.clone(),
            revision_event_id: project.revision_event_id.clone(),
        }
    }
}

/// Project activity operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectActivityOperation {
    /// First Tracker-managed project head.
    Create,
    /// Metadata/workflow update without an address change.
    Update,
    /// Slug/address change.
    Rename,
}

/// Strict append-only M20 project activity content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectActivity {
    /// Tracker schema version.
    pub v: u8,
    /// Activity operation.
    pub op: ProjectActivityOperation,
    /// Immutable project UUID.
    pub project_id: Uuid,
    /// Idempotency request ID.
    pub request_id: Uuid,
    /// Previous snapshot (absent only for create).
    pub before: Option<TrackerProjectSnapshot>,
    /// New snapshot.
    pub after: TrackerProjectSnapshot,
}

/// Parsed project activity event metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerProjectActivity {
    /// Signed event ID.
    pub event_id: String,
    /// Signing pubkey.
    pub author_pubkey: String,
    /// Workspace stream.
    pub workspace_id: Uuid,
    /// Creation timestamp.
    pub created_at: u64,
    /// Strict activity content.
    pub activity: ProjectActivity,
}

/// Project projection diagnostics that do not hide valid heads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProjectDiagnostic {
    /// More than one current signer-owned head claims one immutable UUID.
    DuplicateProjectId {
        /// Immutable project UUID.
        project_id: Uuid,
        /// Canonical event ID.
        canonical: String,
        /// Rejected event ID.
        rejected: String,
    },
    /// More than one signer-owned head claims one slug.
    DuplicateSlug {
        /// Conflicting slug.
        slug: String,
        /// Canonical project UUID.
        canonical: Uuid,
        /// Conflicting project UUID.
        conflicting: Uuid,
    },
    /// An upstream project is referenceable but not manageable by the workspace signer.
    UnmanagedCoordinate {
        /// NIP-MP coordinate that may still be referenced.
        coordinate: String,
    },
    /// A Tracker-tagged head advertises managed identity but fails strict parsing.
    InvalidManagedHead {
        /// NIP-MP coordinate of the invalid head.
        coordinate: String,
        /// Strict parser rejection reason.
        reason: String,
    },
    /// A project head is too far in the future to participate in the projection.
    FutureSkew {
        /// NIP-MP coordinate of the future-dated head.
        coordinate: String,
        /// Signed event timestamp.
        created_at: u64,
        /// Greatest timestamp accepted by this fold.
        max_accepted_at: u64,
    },
}

/// Shared SDK projection of current Tracker-managed projects.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackerProjectProjection {
    /// Canonical current projects keyed by immutable UUID.
    pub projects: BTreeMap<Uuid, TrackerProject>,
    /// Non-fatal identity/slug diagnostics.
    pub diagnostics: Vec<ProjectDiagnostic>,
}

/// Parsed Tracker payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackerPayload {
    /// Issue root.
    Root(IssueRoot),
    /// Issue revision.
    Revision(IssueRevision),
    /// Comment.
    Comment(TrackerComment),
}

/// Strictly parsed Tracker event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerEvent {
    /// Signed event ID.
    pub event_id: String,
    /// Signing pubkey.
    pub author_pubkey: String,
    /// Workspace Tracker stream UUID.
    pub stream_id: Uuid,
    /// Issue UUID.
    pub issue_id: Uuid,
    /// Creation timestamp.
    pub created_at: u64,
    /// Typed payload.
    pub payload: TrackerPayload,
}

/// Workspace-configured Tracker role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerRole {
    /// May create, edit, relate, and comment.
    Member,
    /// Additionally may make terminal/reopen transitions and renumber issues.
    Maintainer,
}

/// Workspace-configured authorization input to the fold.
#[derive(Debug, Clone, Default)]
pub struct TrackerAuthorization {
    roles: HashMap<String, TrackerRole>,
}

impl TrackerAuthorization {
    /// Build authorization from workspace membership configuration.
    pub fn new(roles: impl IntoIterator<Item = (String, TrackerRole)>) -> Self {
        Self {
            roles: roles.into_iter().collect(),
        }
    }

    /// Resolve the configured Tracker role for a pubkey.
    pub fn role(&self, pubkey: &str) -> Option<TrackerRole> {
        self.roles.get(pubkey).copied()
    }
}

/// Current issue projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueProjection {
    /// Workspace stream.
    pub stream_id: Uuid,
    /// Stable issue UUID.
    pub issue_id: Uuid,
    /// Canonical root event.
    pub root_event: String,
    /// Canonical root timestamp used for identifier-collision ordering.
    pub root_created_at: u64,
    /// Canonical revision tip, or the root when unmodified.
    pub tip_event: String,
    /// Creation time of the canonical revision tip.
    pub tip_created_at: u64,
    /// Canonical revision number, with the root defined as zero.
    pub revision: u64,
    /// Time the canonical issue entered its current status.
    pub entered_status_at: u64,
    /// Human-facing identifier.
    pub identifier: String,
    /// Title.
    pub title: String,
    /// Markdown body.
    pub body: String,
    /// State.
    pub status: IssueStatus,
    /// Priority.
    pub priority: Option<Priority>,
    /// Assignee.
    pub assignee: Option<String>,
    /// Project.
    pub project_id: Option<Uuid>,
    /// Due value.
    pub due: Option<Due>,
    /// Estimate in minutes.
    pub estimate_minutes: Option<u32>,
    /// Home channel.
    pub home_channel_id: Option<Uuid>,
    /// Workflow.
    pub workflow: String,
    /// Labels.
    pub labels: BTreeSet<String>,
    /// External references.
    pub external_refs: BTreeSet<ExternalRef>,
    /// Relations.
    pub relations: BTreeSet<Relation>,
}

/// Valid comment retained by the deterministic fold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentProjection {
    /// Comment event ID.
    pub event_id: String,
    /// Signing member pubkey.
    pub author_pubkey: String,
    /// Event creation time.
    pub created_at: u64,
    /// Immutable issue root event ID.
    pub root_event: String,
    /// Optional parent comment event ID.
    pub parent_event: Option<String>,
    /// Markdown comment body.
    pub body: String,
    /// Optional idempotency request ID.
    pub request_id: Option<Uuid>,
    /// Optional import provenance.
    pub import: Option<ImportProvenance>,
}

/// One canonical issue-history entry retained by the deterministic fold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityProjection {
    /// Root or revision event ID.
    pub event_id: String,
    /// Signing workspace member.
    pub author_pubkey: String,
    /// Event creation time.
    pub created_at: u64,
    /// Canonical revision number, with the root defined as zero.
    pub revision: u64,
    /// Optional status-change reason carried by a revision.
    pub reason: Option<String>,
    /// Optional idempotency request ID carried by the source event.
    pub request_id: Option<Uuid>,
    /// Optional import provenance carried by the source event.
    pub import: Option<ImportProvenance>,
    /// Canonical state immediately before this entry, or `None` for the root.
    pub before: Option<IssueProjection>,
    /// Canonical state immediately after this entry.
    pub after: IssueProjection,
}

/// Diagnostic surfaced without mutating canonical state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldDiagnostic {
    /// An event was excluded.
    InvalidEvent {
        /// Excluded event ID.
        event_id: String,
        /// Validation or authorization reason.
        reason: String,
    },
    /// Multiple roots claimed one issue UUID.
    RootCollision {
        /// Colliding issue UUID.
        issue_id: Uuid,
        /// Canonical root event ID.
        canonical: String,
        /// Non-canonical root event ID.
        rejected: String,
    },
    /// Multiple issues claim an identifier.
    IdentifierCollision {
        /// Colliding identifier.
        identifier: String,
        /// Earlier canonical issue UUID.
        canonical: Uuid,
        /// Conflicting issue UUID.
        conflicting: Uuid,
    },
    /// A valid revision lies outside the canonical tip's ancestor chain.
    LosingBranch {
        /// Issue UUID.
        issue_id: Uuid,
        /// Off-chain revision event ID.
        event_id: String,
        /// Winning canonical tip event ID.
        canonical_tip: String,
    },
    /// A relation target/cycle violated graph invariants.
    InvalidRelation {
        /// Source issue UUID.
        issue_id: Uuid,
        /// Relation type.
        relation: RelationType,
        /// Target issue UUID.
        target_issue_id: Uuid,
        /// Validation reason.
        reason: String,
    },
    /// A blocks cycle is retained but surfaced as a deadlock warning.
    BlocksCycle {
        /// One issue participating in the cycle.
        issue_id: Uuid,
        /// Target issue completing the detected cycle.
        target_issue_id: Uuid,
    },
}

/// Complete deterministic fold result.
#[derive(Debug, Clone, Default)]
pub struct TrackerFold {
    /// Current issue projections.
    pub issues: BTreeMap<(Uuid, Uuid), IssueProjection>,
    /// Valid comments ordered by `(created_at, event_id)` per issue.
    pub comments: BTreeMap<(Uuid, Uuid), Vec<CommentProjection>>,
    /// Canonical root-to-tip activity per issue.
    pub activity: BTreeMap<(Uuid, Uuid), Vec<ActivityProjection>>,
    /// Excluded events and surfaced conflicts.
    pub diagnostics: Vec<FoldDiagnostic>,
}

/// Build a strict kind-40020 Tracker issue root.
pub fn build_tracker_issue(stream_id: Uuid, root: &IssueRoot) -> Result<EventBuilder, SdkError> {
    validate_uuid_v4(stream_id, "stream_id")?;
    validate_root(root)?;
    let mut tags = common_tracker_tags(stream_id, root.issue_id, root.request_id)?;
    tags.push(tracker_tag(&["subject", &root.title])?);
    tags.push(tracker_tag(&["status", status_str(root.status)])?);
    tags.push(tracker_tag(&["identifier", &root.identifier])?);
    if let Some(priority) = root.priority {
        tags.push(tracker_tag(&["priority", priority_str(priority)])?);
    }
    if let Some(assignee) = &root.assignee {
        tags.push(tracker_tag(&["p", assignee, "", "assignee"])?);
    }
    if let Some(project_id) = root.project_id {
        tags.push(tracker_tag(&["project", &project_id.to_string()])?);
    }
    if let Some(due) = &root.due {
        tags.push(tracker_tag(&[
            "due",
            &due.value.to_string(),
            due_precision_str(due.precision),
            &due.timezone,
        ])?);
    }
    if let Some(estimate) = root.estimate_minutes {
        tags.push(tracker_tag(&[
            "estimate",
            &estimate.to_string(),
            "minutes",
        ])?);
    }
    if let Some(channel_id) = root.home_channel_id {
        tags.push(tracker_tag(&["home", &channel_id.to_string()])?);
    }
    if let Some(source_event) = &root.source_event_id {
        tags.push(tracker_tag(&["e", source_event, "", "source"])?);
    }
    for label in &root.labels {
        tags.push(tracker_tag(&["l", label])?);
    }
    for external in &root.external_refs {
        tags.push(tracker_tag(&[
            "external_ref",
            &external.kind,
            &external.reference,
        ])?);
    }
    tracker_json_builder(KIND_TRACKER_ISSUE, root, tags)
}

/// Build a strict kind-40021 Tracker issue revision.
pub fn build_tracker_revision(
    stream_id: Uuid,
    revision: &IssueRevision,
) -> Result<EventBuilder, SdkError> {
    validate_uuid_v4(stream_id, "stream_id")?;
    validate_revision(revision)?;
    let mut tags = common_tracker_tags(stream_id, revision.issue_id, revision.request_id)?;
    tags.push(tracker_tag(&["e", &revision.root_event, "", "root"])?);
    tags.push(tracker_tag(&["e", &revision.parent_event, "", "reply"])?);
    tags.push(tracker_tag(&["revision", &revision.revision.to_string()])?);
    if let Some(value) = &revision.set.title {
        tags.push(tracker_tag(&["subject", value])?);
    }
    if let Some(value) = revision.set.status {
        tags.push(tracker_tag(&["status", status_str(value)])?);
    }
    if let Some(value) = &revision.set.identifier {
        tags.push(tracker_tag(&["identifier", value])?);
    }
    if let Some(value) = revision.set.priority {
        tags.push(tracker_tag(&["priority", priority_str(value)])?);
    }
    if let Some(value) = &revision.set.assignee {
        tags.push(tracker_tag(&["p", value, "", "assignee"])?);
    }
    if let Some(value) = revision.set.project_id {
        tags.push(tracker_tag(&["project", &value.to_string()])?);
    }
    if let Some(value) = &revision.set.due {
        tags.push(tracker_tag(&[
            "due",
            &value.value.to_string(),
            due_precision_str(value.precision),
            &value.timezone,
        ])?);
    }
    if let Some(value) = revision.set.estimate_minutes {
        tags.push(tracker_tag(&["estimate", &value.to_string(), "minutes"])?);
    }
    if let Some(value) = revision.set.home_channel_id {
        tags.push(tracker_tag(&["home", &value.to_string()])?);
    }
    if let Some(values) = &revision.set.labels {
        for value in values {
            tags.push(tracker_tag(&["l", value])?);
        }
    }
    for value in &revision.clear {
        tags.push(tracker_tag(&["clear", value])?);
    }
    for (operation, values) in [
        ("add", &revision.relations.add),
        ("remove", &revision.relations.remove),
    ] {
        for value in values {
            tags.push(tracker_tag(&[
                "relation",
                operation,
                relation_str(value.kind),
                &value.target_root_event,
            ])?);
        }
    }
    for (operation, values) in [
        ("add", &revision.external_refs.add),
        ("remove", &revision.external_refs.remove),
    ] {
        for value in values {
            tags.push(tracker_tag(&[
                "external_ref",
                operation,
                &value.kind,
                &value.reference,
            ])?);
        }
    }
    tracker_json_builder(KIND_TRACKER_REVISION, revision, tags)
}

/// Build a strict kind-40022 Tracker issue comment.
pub fn build_tracker_comment(
    stream_id: Uuid,
    root_event: &str,
    parent_event: Option<&str>,
    comment: &IssueComment,
    mentions: &[String],
) -> Result<EventBuilder, SdkError> {
    validate_uuid_v4(stream_id, "stream_id")?;
    validate_comment(comment)?;
    validate_hex(root_event, "root_event")?;
    if let Some(parent_event) = parent_event {
        validate_hex(parent_event, "parent_event")?;
    }
    let mut tags = common_tracker_tags(stream_id, comment.issue_id, comment.request_id)?;
    tags.push(tracker_tag(&["e", root_event, "", "root"])?);
    if let Some(parent_event) = parent_event {
        tags.push(tracker_tag(&["e", parent_event, "", "reply"])?);
    }
    let mut seen = HashSet::new();
    for mention in mentions {
        validate_hex(mention, "mention")?;
        if seen.insert(mention) {
            tags.push(tracker_tag(&["p", mention])?);
        }
    }
    tracker_json_builder(KIND_TRACKER_COMMENT, comment, tags)
}

/// Parse one current kind-30621 head carrying M20 Tracker identity tags.
pub fn parse_tracker_project(event: &Event) -> Result<TrackerProject, SdkError> {
    event.verify().map_err(|error| {
        SdkError::InvalidInput(format!("project signature verification failed: {error}"))
    })?;
    if event.kind.as_u16() as u32 != KIND_PROJECT {
        return invalid("Tracker project must be kind 30621");
    }
    crate::builders::validate_project_envelope(
        &event.tags.iter().cloned().collect::<Vec<_>>(),
        &event.content,
    )?;
    let slug = singleton_string_tag(event, "d")?;
    let project_id = singleton_uuid_value_tag(event, "buzz-project-id", &[4, 5])?;
    let workspace_id = singleton_uuid_value_tag(event, "buzz-workspace", &[4])?;
    let name = optional_string_tag(event, "name")?.unwrap_or_else(|| slug.clone());
    if name.trim().is_empty() || name.len() > 256 {
        return invalid("project name must be 1..=256 bytes");
    }
    let status_tags = tag_values(event, "buzz-status");
    let statuses = if status_tags.is_empty() {
        default_project_statuses()
    } else {
        let mut values = Vec::with_capacity(status_tags.len());
        let mut seen = HashSet::new();
        for tag in status_tags {
            if tag.len() != 2 {
                return invalid("buzz-status tags must contain exactly one value");
            }
            let status = IssueStatus::from_str(&tag[1])?;
            if !seen.insert(status) {
                return invalid("buzz-status values must be unique");
            }
            values.push(status);
        }
        values
    };
    let default_channel_id = optional_string_tag(event, "buzz-channel")?
        .map(|value| parse_canonical_uuid(&value, "buzz-channel", &[4]))
        .transpose()?;
    let default_verify = optional_string_tag(event, "buzz-default-verify")?;
    if default_verify
        .as_ref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return invalid("buzz-default-verify cannot be empty");
    }
    Ok(TrackerProject {
        project_id,
        workspace_id,
        slug,
        name,
        statuses,
        default_channel_id,
        default_verify,
        author_pubkey: event.pubkey.to_hex(),
        revision_event_id: event.id.to_hex(),
        updated_at: event.created_at.as_secs(),
    })
}

/// Fold current project heads by immutable UUID and surface collisions.
pub fn fold_tracker_projects(
    events: impl IntoIterator<Item = Event>,
    workspace_id: Uuid,
    managed_signer: &str,
) -> TrackerProjectProjection {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    fold_tracker_projects_at(events, workspace_id, managed_signer, now)
}

/// Fold current project heads at a caller-supplied clock for deterministic verification.
pub fn fold_tracker_projects_at(
    events: impl IntoIterator<Item = Event>,
    workspace_id: Uuid,
    managed_signer: &str,
    now: u64,
) -> TrackerProjectProjection {
    let mut projection = TrackerProjectProjection::default();
    let mut parsed = Vec::new();
    for event in events {
        let coordinate = tag_values(&event, "d")
            .first()
            .and_then(|tag| tag.get(1))
            .map(|slug| format!("30621:{}:{slug}", event.pubkey.to_hex()));
        if event.kind.as_u16() as u32 == KIND_PROJECT
            && event.created_at.as_secs() > now.saturating_add(MAX_FUTURE_SKEW_SECONDS)
        {
            if let Some(coordinate) = coordinate {
                projection.diagnostics.push(ProjectDiagnostic::FutureSkew {
                    coordinate,
                    created_at: event.created_at.as_secs(),
                    max_accepted_at: now.saturating_add(MAX_FUTURE_SKEW_SECONDS),
                });
            }
            continue;
        }
        match parse_tracker_project(&event) {
            Ok(project) if project.workspace_id == workspace_id => parsed.push(project),
            Err(error) if event.kind.as_u16() as u32 == KIND_PROJECT => {
                if let Some(coordinate) = coordinate {
                    if !tag_values(&event, "buzz-project-id").is_empty()
                        || !tag_values(&event, "buzz-workspace").is_empty()
                    {
                        projection
                            .diagnostics
                            .push(ProjectDiagnostic::InvalidManagedHead {
                                coordinate,
                                reason: error.to_string(),
                            });
                    } else {
                        projection
                            .diagnostics
                            .push(ProjectDiagnostic::UnmanagedCoordinate { coordinate });
                    }
                }
            }
            _ => {}
        }
    }
    parsed.sort_by(|left, right| {
        (left.updated_at, left.revision_event_id.as_str())
            .cmp(&(right.updated_at, right.revision_event_id.as_str()))
    });
    for project in parsed {
        if project.author_pubkey != managed_signer {
            projection
                .diagnostics
                .push(ProjectDiagnostic::UnmanagedCoordinate {
                    coordinate: format!("30621:{}:{}", project.author_pubkey, project.slug),
                });
            continue;
        }
        if let Some(previous) = projection
            .projects
            .insert(project.project_id, project.clone())
        {
            projection
                .diagnostics
                .push(ProjectDiagnostic::DuplicateProjectId {
                    project_id: project.project_id,
                    canonical: project.revision_event_id.clone(),
                    rejected: previous.revision_event_id,
                });
        }
    }
    let mut slug_groups = BTreeMap::<String, Vec<&TrackerProject>>::new();
    for project in projection.projects.values() {
        slug_groups
            .entry(project.slug.to_ascii_lowercase())
            .or_default()
            .push(project);
    }
    for mut projects in slug_groups.into_values().filter(|group| group.len() > 1) {
        projects.sort_by(|left, right| {
            (left.updated_at, left.revision_event_id.as_str())
                .cmp(&(right.updated_at, right.revision_event_id.as_str()))
        });
        let canonical = projects.pop().expect("duplicate slug group is non-empty");
        projection
            .diagnostics
            .extend(
                projects
                    .into_iter()
                    .map(|project| ProjectDiagnostic::DuplicateSlug {
                        slug: canonical.slug.clone(),
                        canonical: canonical.project_id,
                        conflicting: project.project_id,
                    }),
            );
    }
    projection
}

/// Build one immutable kind-40025 project activity event.
pub fn build_tracker_project_activity(
    workspace_id: Uuid,
    activity: &ProjectActivity,
    project_author: &str,
) -> Result<EventBuilder, SdkError> {
    validate_uuid_v4(workspace_id, "workspace_id")?;
    validate_project_activity(activity)?;
    validate_hex(project_author, "project_author")?;
    let coordinate = format!("{KIND_PROJECT}:{project_author}:{}", activity.after.slug);
    let tags = vec![
        tracker_tag(&["h", &workspace_id.to_string()])?,
        tracker_tag(&["d", &activity.project_id.to_string()])?,
        tracker_tag(&["t", "buzz-tracker-project"])?,
        tracker_tag(&["request", &activity.request_id.to_string()])?,
        tracker_tag(&["a", &coordinate])?,
    ];
    tracker_json_builder(KIND_TRACKER_PROJECT_ACTIVITY, activity, tags)
}

/// Parse and validate one immutable project activity event.
pub fn parse_tracker_project_activity(
    event: &Event,
    now: u64,
) -> Result<TrackerProjectActivity, SdkError> {
    event.verify().map_err(|error| {
        SdkError::InvalidInput(format!(
            "project activity signature verification failed: {error}"
        ))
    })?;
    if event.kind.as_u16() as u32 != KIND_TRACKER_PROJECT_ACTIVITY {
        return invalid("project activity must be kind 40025");
    }
    if event.created_at.as_secs() > now.saturating_add(MAX_FUTURE_SKEW_SECONDS) {
        return invalid("Tracker event timestamp exceeds 300-second future-skew limit");
    }
    validate_allowed_tags(event, &["h", "d", "t", "request", "a", "auth"])?;
    let workspace_id = singleton_uuid_value_tag(event, "h", &[4])?;
    let project_id = singleton_uuid_value_tag(event, "d", &[4, 5])?;
    let request_id = singleton_uuid_value_tag(event, "request", &[4, 5])?;
    require_exact_tag(event, "t", "buzz-tracker-project")?;
    let activity: ProjectActivity = strict_json(&event.content)?;
    validate_project_activity(&activity)?;
    if activity.project_id != project_id || activity.request_id != request_id {
        return invalid("project activity content disagrees with d/request tags");
    }
    let expected_coordinate = format!(
        "{KIND_PROJECT}:{}:{}",
        event.pubkey.to_hex(),
        activity.after.slug
    );
    require_exact_tag(event, "a", &expected_coordinate)?;
    Ok(TrackerProjectActivity {
        event_id: event.id.to_hex(),
        author_pubkey: event.pubkey.to_hex(),
        workspace_id,
        created_at: event.created_at.as_secs(),
        activity,
    })
}

/// Parse and validate a signed Tracker event against current wall time.
pub fn parse_tracker_event(event: &Event, now: u64) -> Result<TrackerEvent, SdkError> {
    event.verify().map_err(|error| {
        SdkError::InvalidInput(format!(
            "Tracker event signature verification failed: {error}"
        ))
    })?;
    let kind = event.kind.as_u16() as u32;
    if !matches!(
        kind,
        KIND_TRACKER_ISSUE | KIND_TRACKER_REVISION | KIND_TRACKER_COMMENT
    ) {
        return invalid("unsupported Tracker event kind");
    }
    let created_at = event.created_at.as_secs();
    if created_at > now.saturating_add(MAX_FUTURE_SKEW_SECONDS) {
        return invalid("Tracker event timestamp exceeds 300-second future-skew limit");
    }
    let stream_id = singleton_uuid_value_tag(event, "h", &[4])?;
    let issue_id = singleton_uuid_value_tag(event, "d", &[4, 5])?;
    require_discovery_tag(event)?;
    let request_tag = optional_uuid_value_tag(event, "request", &[4, 5])?;
    let payload = match kind {
        KIND_TRACKER_ISSUE => {
            validate_allowed_tags(
                event,
                &[
                    "h",
                    "d",
                    "t",
                    "subject",
                    "status",
                    "identifier",
                    "priority",
                    "p",
                    "project",
                    "due",
                    "estimate",
                    "home",
                    "e",
                    "l",
                    "request",
                    "external_ref",
                    "auth",
                ],
            )?;
            let root: IssueRoot = strict_json(&event.content)?;
            validate_root(&root)?;
            if root.issue_id != issue_id || root.request_id != request_tag {
                return invalid("root content disagrees with d/request tags");
            }
            validate_root_projection_tags(event, &root)?;
            TrackerPayload::Root(root)
        }
        KIND_TRACKER_REVISION => {
            validate_allowed_tags(
                event,
                &[
                    "h",
                    "d",
                    "t",
                    "e",
                    "revision",
                    "subject",
                    "status",
                    "identifier",
                    "priority",
                    "p",
                    "project",
                    "due",
                    "estimate",
                    "home",
                    "l",
                    "request",
                    "clear",
                    "relation",
                    "external_ref",
                    "auth",
                ],
            )?;
            let revision: IssueRevision = strict_json(&event.content)?;
            validate_revision(&revision)?;
            if revision.issue_id != issue_id || revision.request_id != request_tag {
                return invalid("revision content disagrees with d/request tags");
            }
            validate_revision_identity_tags(event, &revision)?;
            TrackerPayload::Revision(revision)
        }
        KIND_TRACKER_COMMENT => {
            validate_allowed_tags(event, &["h", "d", "t", "e", "p", "request", "auth"])?;
            let comment: IssueComment = strict_json(&event.content)?;
            validate_comment(&comment)?;
            if comment.issue_id != issue_id || comment.request_id != request_tag {
                return invalid("comment content disagrees with d/request tags");
            }
            let root_event = require_single_marked_event_tag(event, "root")?;
            let parent_event = optional_single_marked_event_tag(event, "reply")?;
            if tag_values(event, "e").len() != usize::from(parent_event.is_some()) + 1 {
                return invalid("comment e tags must be one root and at most one reply");
            }
            TrackerPayload::Comment(TrackerComment {
                content: comment,
                root_event,
                parent_event,
            })
        }
        _ => return invalid("unsupported Tracker event kind"),
    };
    Ok(TrackerEvent {
        event_id: event.id.to_hex(),
        author_pubkey: event.pubkey.to_hex(),
        stream_id,
        issue_id,
        created_at,
        payload,
    })
}

/// Fold valid Tracker events using workspace-configured roles.
pub fn fold_tracker_events(
    events: impl IntoIterator<Item = TrackerEvent>,
    authorization: &TrackerAuthorization,
) -> TrackerFold {
    let mut result = TrackerFold::default();
    let mut grouped: BTreeMap<(Uuid, Uuid), Vec<TrackerEvent>> = BTreeMap::new();
    let mut seen = HashSet::new();
    for event in events {
        if !seen.insert(event.event_id.clone()) {
            continue;
        }
        grouped
            .entry((event.stream_id, event.issue_id))
            .or_default()
            .push(event);
    }
    for (key, events) in grouped {
        fold_one_issue(key, events, authorization, &mut result);
    }
    validate_relation_graph(&mut result);
    surface_identifier_collisions(&mut result);
    result
}

fn fold_one_issue(
    key: (Uuid, Uuid),
    events: Vec<TrackerEvent>,
    authorization: &TrackerAuthorization,
    result: &mut TrackerFold,
) {
    for event in events
        .iter()
        .filter(|event| matches!(event.payload, TrackerPayload::Root(_)))
        .filter(|event| authorization.role(&event.author_pubkey).is_none())
    {
        result.diagnostics.push(FoldDiagnostic::InvalidEvent {
            event_id: event.event_id.clone(),
            reason: "root author is not a workspace member".into(),
        });
    }
    let mut roots: Vec<&TrackerEvent> = events
        .iter()
        .filter(|event| matches!(event.payload, TrackerPayload::Root(_)))
        .filter(|event| authorization.role(&event.author_pubkey).is_some())
        .collect();
    roots.sort_by_key(|event| (event.created_at, event.event_id.as_str()));
    let Some(root_event) = roots.first().copied() else {
        for event in events {
            result.diagnostics.push(FoldDiagnostic::InvalidEvent {
                event_id: event.event_id,
                reason: "issue has no member-authored root".into(),
            });
        }
        return;
    };
    for collision in roots.iter().skip(1) {
        result.diagnostics.push(FoldDiagnostic::RootCollision {
            issue_id: key.1,
            canonical: root_event.event_id.clone(),
            rejected: collision.event_id.clone(),
        });
    }
    let TrackerPayload::Root(root) = &root_event.payload else {
        return;
    };
    if root.status.is_terminal()
        && authorization.role(&root_event.author_pubkey) != Some(TrackerRole::Maintainer)
    {
        result.diagnostics.push(FoldDiagnostic::InvalidEvent {
            event_id: root_event.event_id.clone(),
            reason: "terminal issue creation requires maintainer role".into(),
        });
        return;
    }
    let initial = projection_from_root(
        key.0,
        root_event.event_id.clone(),
        root_event.created_at,
        root,
    );
    fold_comments(key, &events, root_event, authorization, result);
    let revisions: BTreeMap<String, &TrackerEvent> = events
        .iter()
        .filter(|event| matches!(event.payload, TrackerPayload::Revision(_)))
        .map(|event| (event.event_id.clone(), event))
        .collect();
    let mut memo: HashMap<String, Result<IssueProjection, String>> = HashMap::new();
    let mut valid = Vec::new();
    for event in revisions.values() {
        let mut visiting = HashSet::new();
        match evaluate_revision(
            event,
            root_event,
            &initial,
            &revisions,
            authorization,
            &mut memo,
            &mut visiting,
        ) {
            Ok(state) => valid.push((*event, state)),
            Err(reason) => result.diagnostics.push(FoldDiagnostic::InvalidEvent {
                event_id: event.event_id.clone(),
                reason,
            }),
        }
    }
    let winner = valid
        .iter()
        .max_by_key(|(event, _)| (event.created_at, event.event_id.as_str()));
    let mut projection = if let Some((winner_event, state)) = winner {
        let canonical_chain: HashSet<&str> = revision_chain(root_event, winner_event, &revisions)
            .into_iter()
            .map(|event| event.event_id.as_str())
            .collect();
        for (event, _) in &valid {
            if !canonical_chain.contains(event.event_id.as_str()) {
                result.diagnostics.push(FoldDiagnostic::LosingBranch {
                    issue_id: key.1,
                    event_id: event.event_id.clone(),
                    canonical_tip: winner_event.event_id.clone(),
                });
            }
        }
        state.clone()
    } else {
        initial.clone()
    };
    let activity = canonical_activity(root_event, &initial, winner, &revisions, &memo);
    // External refs are operation-based, not a whole-list scalar. Apply every
    // structurally and semantically valid branch operation in deterministic
    // event order so concurrent adds cannot clobber one another.
    projection.external_refs = initial.external_refs;
    valid.sort_by_key(|(event, _)| (event.created_at, event.event_id.as_str()));
    for (event, _) in &valid {
        let TrackerPayload::Revision(revision) = &event.payload else {
            continue;
        };
        for value in &revision.external_refs.remove {
            projection.external_refs.remove(value);
        }
        for value in &revision.external_refs.add {
            projection.external_refs.insert(value.clone());
        }
    }
    result.activity.insert(key, activity);
    result.issues.insert(key, projection);
}

fn canonical_activity(
    root_event: &TrackerEvent,
    initial: &IssueProjection,
    winner: Option<&(&TrackerEvent, IssueProjection)>,
    revisions: &BTreeMap<String, &TrackerEvent>,
    memo: &HashMap<String, Result<IssueProjection, String>>,
) -> Vec<ActivityProjection> {
    let mut activity = vec![ActivityProjection {
        event_id: root_event.event_id.clone(),
        author_pubkey: root_event.author_pubkey.clone(),
        created_at: root_event.created_at,
        revision: 0,
        reason: None,
        request_id: match &root_event.payload {
            TrackerPayload::Root(root) => root.request_id,
            _ => None,
        },
        import: match &root_event.payload {
            TrackerPayload::Root(root) => root.import.clone(),
            _ => None,
        },
        before: None,
        after: initial.clone(),
    }];
    let Some((winner_event, _)) = winner else {
        return activity;
    };
    for event in revision_chain(root_event, winner_event, revisions) {
        let TrackerPayload::Revision(revision) = &event.payload else {
            continue;
        };
        let Some(Ok(after)) = memo.get(&event.event_id) else {
            continue;
        };
        let before = if revision.parent_event == root_event.event_id {
            initial.clone()
        } else {
            let Some(Ok(parent)) = memo.get(&revision.parent_event) else {
                continue;
            };
            parent.clone()
        };
        activity.push(ActivityProjection {
            event_id: event.event_id.clone(),
            author_pubkey: event.author_pubkey.clone(),
            created_at: event.created_at,
            revision: revision.revision,
            reason: revision.reason.clone(),
            request_id: revision.request_id,
            import: revision.import.clone(),
            before: Some(before),
            after: after.clone(),
        });
    }
    activity
}

fn revision_chain<'a>(
    root_event: &TrackerEvent,
    tip: &'a TrackerEvent,
    revisions: &BTreeMap<String, &'a TrackerEvent>,
) -> Vec<&'a TrackerEvent> {
    let mut chain = Vec::new();
    let mut current = tip;
    loop {
        chain.push(current);
        let TrackerPayload::Revision(revision) = &current.payload else {
            break;
        };
        if revision.parent_event == root_event.event_id {
            break;
        }
        let Some(parent) = revisions.get(&revision.parent_event) else {
            break;
        };
        current = parent;
    }
    chain.reverse();
    chain
}

#[allow(clippy::too_many_arguments)]
fn evaluate_revision(
    event: &TrackerEvent,
    root_event: &TrackerEvent,
    initial: &IssueProjection,
    revisions: &BTreeMap<String, &TrackerEvent>,
    authorization: &TrackerAuthorization,
    memo: &mut HashMap<String, Result<IssueProjection, String>>,
    visiting: &mut HashSet<String>,
) -> Result<IssueProjection, String> {
    if let Some(cached) = memo.get(&event.event_id) {
        return cached.clone();
    }
    if !visiting.insert(event.event_id.clone()) {
        return Err("revision parent cycle".into());
    }
    let TrackerPayload::Revision(revision) = &event.payload else {
        return Err("not a revision".into());
    };
    if revision.root_event != root_event.event_id {
        return Err("revision names a non-canonical root".into());
    }
    let (mut parent, parent_revision) = if revision.parent_event == root_event.event_id {
        (initial.clone(), 0)
    } else {
        let parent_event = revisions
            .get(&revision.parent_event)
            .ok_or_else(|| "revision parent not found".to_string())?;
        let state = evaluate_revision(
            parent_event,
            root_event,
            initial,
            revisions,
            authorization,
            memo,
            visiting,
        )?;
        let TrackerPayload::Revision(parent_payload) = &parent_event.payload else {
            return Err("revision parent is not a revision".into());
        };
        (state, parent_payload.revision)
    };
    if revision.revision != parent_revision + 1 {
        return Err("revision integer does not equal parent revision plus one".into());
    }
    let role = authorization
        .role(&event.author_pubkey)
        .ok_or_else(|| "revision author is not a workspace member".to_string())?;
    let renumber = revision
        .set
        .identifier
        .as_ref()
        .is_some_and(|value| value != &parent.identifier);
    let status_change = revision
        .set
        .status
        .filter(|status| *status != parent.status);
    let restricted_status =
        status_change.is_some_and(|next| parent.status.is_terminal() || next.is_terminal());
    if (renumber || restricted_status) && role != TrackerRole::Maintainer {
        return Err("renumber and terminal/reopen transitions require maintainer role".into());
    }
    let prior_status = parent.status;
    apply_revision(&mut parent, revision)?;
    parent.tip_event = event.event_id.clone();
    parent.tip_created_at = event.created_at;
    parent.revision = revision.revision;
    if parent.status != prior_status {
        parent.entered_status_at = event.created_at;
    }
    visiting.remove(&event.event_id);
    memo.insert(event.event_id.clone(), Ok(parent.clone()));
    Ok(parent)
}

fn fold_comments(
    key: (Uuid, Uuid),
    events: &[TrackerEvent],
    root_event: &TrackerEvent,
    authorization: &TrackerAuthorization,
    result: &mut TrackerFold,
) {
    let comment_ids: HashSet<&str> = events
        .iter()
        .filter_map(|event| match &event.payload {
            TrackerPayload::Comment(comment)
                if authorization.role(&event.author_pubkey).is_some()
                    && comment.root_event == root_event.event_id =>
            {
                Some(event.event_id.as_str())
            }
            _ => None,
        })
        .collect();
    let mut comments = Vec::new();
    for event in events {
        let TrackerPayload::Comment(comment) = &event.payload else {
            continue;
        };
        let reason = if authorization.role(&event.author_pubkey).is_none() {
            Some("comment author is not a workspace member")
        } else if comment.root_event != root_event.event_id {
            Some("comment names a non-canonical root")
        } else if comment
            .parent_event
            .as_deref()
            .is_some_and(|parent| !comment_ids.contains(parent))
        {
            Some("comment reply parent not found in the same issue")
        } else {
            None
        };
        if let Some(reason) = reason {
            result.diagnostics.push(FoldDiagnostic::InvalidEvent {
                event_id: event.event_id.clone(),
                reason: reason.into(),
            });
            continue;
        }
        comments.push(CommentProjection {
            event_id: event.event_id.clone(),
            author_pubkey: event.author_pubkey.clone(),
            created_at: event.created_at,
            root_event: comment.root_event.clone(),
            parent_event: comment.parent_event.clone(),
            body: comment.content.body.clone(),
            request_id: comment.content.request_id,
            import: comment.content.import.clone(),
        });
    }
    comments.sort_by(|left, right| {
        (left.created_at, left.event_id.as_str()).cmp(&(right.created_at, right.event_id.as_str()))
    });
    if !comments.is_empty() {
        result.comments.insert(key, comments);
    }
}

fn apply_revision(state: &mut IssueProjection, revision: &IssueRevision) -> Result<(), String> {
    if let Some(value) = &revision.set.identifier {
        state.identifier = value.clone();
    }
    if let Some(value) = &revision.set.title {
        state.title = value.clone();
    }
    if let Some(value) = &revision.set.body {
        state.body = value.clone();
    }
    if let Some(value) = revision.set.status {
        if revision
            .reason
            .as_deref()
            .is_some_and(|reason| reason.trim().is_empty())
        {
            return Err("status change reason must be non-empty when present".into());
        }
        state.status = value;
    }
    if let Some(value) = revision.set.priority {
        state.priority = Some(value);
    }
    if let Some(value) = &revision.set.assignee {
        state.assignee = Some(value.clone());
    }
    if let Some(value) = revision.set.project_id {
        state.project_id = Some(value);
    }
    if let Some(value) = &revision.set.due {
        validate_due(value).map_err(|error| error.to_string())?;
        state.due = Some(value.clone());
    }
    if let Some(value) = revision.set.estimate_minutes {
        state.estimate_minutes = Some(value);
    }
    if let Some(value) = revision.set.home_channel_id {
        state.home_channel_id = Some(value);
    }
    if let Some(value) = &revision.set.workflow {
        state.workflow = value.clone();
    }
    if let Some(values) = &revision.set.labels {
        state.labels = validate_labels(values).map_err(|error| error.to_string())?;
    }
    for field in &revision.clear {
        match field.as_str() {
            "priority" => state.priority = None,
            "assignee" => state.assignee = None,
            "project_id" => state.project_id = None,
            "due" => state.due = None,
            "estimate_minutes" => state.estimate_minutes = None,
            "home_channel_id" => state.home_channel_id = None,
            _ => return Err(format!("field {field:?} cannot be cleared")),
        }
    }
    for value in &revision.external_refs.remove {
        state.external_refs.remove(value);
    }
    for value in &revision.external_refs.add {
        state.external_refs.insert(value.clone());
    }
    for value in &revision.relations.remove {
        state.relations.remove(value);
    }
    for value in &revision.relations.add {
        if value.target_issue_id == state.issue_id || value.target_root_event == state.root_event {
            return Err("self relation is invalid".into());
        }
        state.relations.insert(value.clone());
    }
    Ok(())
}

fn projection_from_root(
    stream_id: Uuid,
    root_event: String,
    root_created_at: u64,
    root: &IssueRoot,
) -> IssueProjection {
    IssueProjection {
        stream_id,
        issue_id: root.issue_id,
        tip_event: root_event.clone(),
        tip_created_at: root_created_at,
        revision: 0,
        entered_status_at: root_created_at,
        root_event,
        root_created_at,
        identifier: root.identifier.clone(),
        title: root.title.clone(),
        body: root.body.clone(),
        status: root.status,
        priority: root.priority,
        assignee: root.assignee.clone(),
        project_id: root.project_id,
        due: root.due.clone(),
        estimate_minutes: root.estimate_minutes,
        home_channel_id: root.home_channel_id,
        workflow: root.workflow.clone(),
        labels: root.labels.iter().cloned().collect(),
        external_refs: root.external_refs.iter().cloned().collect(),
        relations: BTreeSet::new(),
    }
}

fn surface_identifier_collisions(result: &mut TrackerFold) {
    type IdentifierClaim = ((Uuid, Uuid), (u64, String));
    let mut identifiers: BTreeMap<(Uuid, String), Vec<IdentifierClaim>> = BTreeMap::new();
    for (key, issue) in &result.issues {
        identifiers
            .entry((key.0, issue.identifier.clone()))
            .or_default()
            .push((*key, (issue.root_created_at, issue.root_event.clone())));
    }
    for ((_stream, identifier), mut claims) in identifiers {
        if claims.len() < 2 {
            continue;
        }
        claims.sort_by(|left, right| left.1.cmp(&right.1));
        let canonical = claims[0].0 .1;
        for claim in claims.iter().skip(1) {
            result
                .diagnostics
                .push(FoldDiagnostic::IdentifierCollision {
                    identifier: identifier.clone(),
                    canonical,
                    conflicting: claim.0 .1,
                });
        }
    }
}

fn validate_relation_graph(result: &mut TrackerFold) {
    let roots: HashMap<(Uuid, Uuid), String> = result
        .issues
        .iter()
        .map(|(key, issue)| (*key, issue.root_event.clone()))
        .collect();
    let mut invalid = Vec::new();
    for (key, issue) in &result.issues {
        for relation in &issue.relations {
            let target = (key.0, relation.target_issue_id);
            if roots.get(&target) != Some(&relation.target_root_event) {
                invalid.push((
                    *key,
                    relation.clone(),
                    "target is not a root in the same stream".to_string(),
                ));
            }
        }
    }
    for (key, relation, reason) in invalid {
        if let Some(issue) = result.issues.get_mut(&key) {
            issue.relations.remove(&relation);
        }
        result.diagnostics.push(FoldDiagnostic::InvalidRelation {
            issue_id: key.1,
            relation: relation.kind,
            target_issue_id: relation.target_issue_id,
            reason,
        });
    }

    for kind in [
        RelationType::Parent,
        RelationType::DuplicateOf,
        RelationType::Blocks,
    ] {
        let edges: Vec<_> = result
            .issues
            .iter()
            .flat_map(|(key, issue)| {
                issue
                    .relations
                    .iter()
                    .filter(move |relation| relation.kind == kind)
                    .map(move |relation| (*key, relation.clone()))
            })
            .collect();
        let adjacency: HashMap<(Uuid, Uuid), Vec<(Uuid, Uuid)>> =
            edges
                .iter()
                .fold(HashMap::new(), |mut graph, (source, relation)| {
                    graph
                        .entry(*source)
                        .or_default()
                        .push((source.0, relation.target_issue_id));
                    graph
                });
        let mut cycle_edges = Vec::new();
        for (source, relation) in &edges {
            let target = (source.0, relation.target_issue_id);
            if graph_reaches(&adjacency, target, *source, &mut HashSet::new()) {
                cycle_edges.push((*source, relation.clone()));
            }
        }
        cycle_edges.sort_by_key(|(source, relation)| (source.1, relation.target_issue_id));
        cycle_edges.dedup();
        for (source, relation) in cycle_edges {
            if kind == RelationType::Blocks {
                result.diagnostics.push(FoldDiagnostic::BlocksCycle {
                    issue_id: source.1,
                    target_issue_id: relation.target_issue_id,
                });
            } else {
                if let Some(issue) = result.issues.get_mut(&source) {
                    issue.relations.remove(&relation);
                }
                result.diagnostics.push(FoldDiagnostic::InvalidRelation {
                    issue_id: source.1,
                    relation: kind,
                    target_issue_id: relation.target_issue_id,
                    reason: "relation cycle is invalid".into(),
                });
            }
        }
    }
}

fn graph_reaches(
    graph: &HashMap<(Uuid, Uuid), Vec<(Uuid, Uuid)>>,
    current: (Uuid, Uuid),
    target: (Uuid, Uuid),
    visiting: &mut HashSet<(Uuid, Uuid)>,
) -> bool {
    if current == target {
        return true;
    }
    if !visiting.insert(current) {
        return false;
    }
    graph.get(&current).is_some_and(|next| {
        next.iter()
            .any(|candidate| graph_reaches(graph, *candidate, target, visiting))
    })
}

fn tracker_tag(parts: &[&str]) -> Result<Tag, SdkError> {
    Tag::parse(parts.iter().copied()).map_err(|error| SdkError::InvalidTag(error.to_string()))
}

fn tracker_json_builder<T: Serialize>(
    kind: u32,
    content: &T,
    tags: Vec<Tag>,
) -> Result<EventBuilder, SdkError> {
    let content = serde_json::to_string(content).map_err(|error| {
        SdkError::InvalidInput(format!("Tracker content serialization failed: {error}"))
    })?;
    if content.len() > 64 * 1024 {
        return Err(SdkError::ContentTooLarge {
            max: 64 * 1024,
            got: content.len(),
        });
    }
    // Tracker `p` tags are semantic projections (for example, the issue
    // assignee), so a self-assignment must survive nostr-sdk's default
    // self-tag filtering and remain consistent with the signed JSON payload.
    Ok(EventBuilder::new(Kind::Custom(kind as u16), content)
        .tags(tags)
        .allow_self_tagging())
}

fn common_tracker_tags(
    stream_id: Uuid,
    issue_id: Uuid,
    request_id: Option<Uuid>,
) -> Result<Vec<Tag>, SdkError> {
    let mut tags = vec![
        tracker_tag(&["h", &stream_id.to_string()])?,
        tracker_tag(&["d", &issue_id.to_string()])?,
        tracker_tag(&["t", TRACKER_DISCOVERY_TAG])?,
    ];
    if let Some(request_id) = request_id {
        tags.push(tracker_tag(&["request", &request_id.to_string()])?);
    }
    Ok(tags)
}

/// Built-in ordered workflow used when a project omits explicit statuses.
pub fn default_project_statuses() -> Vec<IssueStatus> {
    vec![
        IssueStatus::Triage,
        IssueStatus::Backlog,
        IssueStatus::Todo,
        IssueStatus::InProgress,
        IssueStatus::InReview,
        IssueStatus::Done,
        IssueStatus::Cancelled,
    ]
}

fn validate_project_snapshot(snapshot: &TrackerProjectSnapshot) -> Result<(), SdkError> {
    validate_uuid_versions(snapshot.project_id, "project_id", &[4, 5])?;
    if snapshot.slug.trim().is_empty() || snapshot.slug.len() > 1024 {
        return invalid("project slug must be 1..=1024 bytes");
    }
    if snapshot.name.trim().is_empty() || snapshot.name.len() > 256 {
        return invalid("project name must be 1..=256 bytes");
    }
    if snapshot.statuses.is_empty() {
        return invalid("project statuses cannot be empty");
    }
    let statuses: HashSet<_> = snapshot.statuses.iter().copied().collect();
    if statuses.len() != snapshot.statuses.len() {
        return invalid("project statuses must be unique");
    }
    if let Some(channel_id) = snapshot.default_channel_id {
        validate_uuid_v4(channel_id, "default_channel_id")?;
    }
    if snapshot
        .default_verify
        .as_ref()
        .is_some_and(|value| value.trim().is_empty() || value.len() > 2048)
    {
        return invalid("default_verify must be 1..=2048 bytes when set");
    }
    validate_hex(&snapshot.revision_event_id, "revision_event_id")?;
    Ok(())
}

fn validate_project_activity(activity: &ProjectActivity) -> Result<(), SdkError> {
    if activity.v != TRACKER_VERSION {
        return invalid("invalid Tracker project activity version");
    }
    validate_uuid_versions(activity.project_id, "project_id", &[4, 5])?;
    validate_uuid_versions(activity.request_id, "request_id", &[4, 5])?;
    validate_project_snapshot(&activity.after)?;
    if activity.after.project_id != activity.project_id {
        return invalid("project activity after snapshot disagrees with project_id");
    }
    if let Some(before) = &activity.before {
        validate_project_snapshot(before)?;
        if before.project_id != activity.project_id {
            return invalid("project activity before snapshot disagrees with project_id");
        }
    }
    match activity.op {
        ProjectActivityOperation::Create if activity.before.is_some() => {
            invalid("project create activity cannot have a before snapshot")
        }
        ProjectActivityOperation::Update
            if activity
                .before
                .as_ref()
                .is_none_or(|before| before.slug != activity.after.slug) =>
        {
            invalid("project update activity requires unchanged slug")
        }
        ProjectActivityOperation::Rename
            if activity
                .before
                .as_ref()
                .is_none_or(|before| before.slug == activity.after.slug) =>
        {
            invalid("project rename activity requires a changed slug")
        }
        _ => Ok(()),
    }
}

fn validate_root(root: &IssueRoot) -> Result<(), SdkError> {
    if root.v != TRACKER_VERSION || root.op != "create" {
        return invalid("invalid Tracker root version/op");
    }
    validate_uuid_versions(root.issue_id, "issue_id", &[4, 5])?;
    validate_identifier(&root.identifier)?;
    validate_title(&root.title)?;
    validate_optional_pubkey(root.assignee.as_deref())?;
    if root.workflow.trim().is_empty() {
        return invalid("workflow cannot be empty");
    }
    if let Some(due) = &root.due {
        validate_due(due)?;
    }
    if root.estimate_minutes == Some(0) {
        return invalid("estimate_minutes must be positive");
    }
    validate_labels(&root.labels)?;
    validate_external_refs(&root.external_refs)?;
    if let Some(source_event_id) = &root.source_event_id {
        validate_hex(source_event_id, "source_event_id")?;
    }
    if let Some(request_id) = root.request_id {
        validate_uuid_versions(request_id, "request_id", &[4, 5])?;
    }
    if let Some(import) = &root.import {
        validate_import_provenance(import)?;
    }
    Ok(())
}

fn validate_revision(revision: &IssueRevision) -> Result<(), SdkError> {
    if revision.v != TRACKER_VERSION || revision.op != "revise" || revision.revision == 0 {
        return invalid("invalid Tracker revision version/op/revision");
    }
    validate_hex(&revision.root_event, "root_event")?;
    validate_hex(&revision.parent_event, "parent_event")?;
    validate_uuid_versions(revision.issue_id, "issue_id", &[4, 5])?;
    if let Some(request_id) = revision.request_id {
        validate_uuid_versions(request_id, "request_id", &[4, 5])?;
    }
    if let Some(import) = &revision.import {
        validate_import_provenance(import)?;
    }
    if let Some(value) = &revision.set.identifier {
        validate_identifier(value)?;
    }
    if let Some(value) = &revision.set.title {
        validate_title(value)?;
    }
    validate_optional_pubkey(revision.set.assignee.as_deref())?;
    if let Some(value) = &revision.set.due {
        validate_due(value)?;
    }
    if revision.set.estimate_minutes == Some(0) {
        return invalid("estimate_minutes must be positive");
    }
    if let Some(labels) = &revision.set.labels {
        validate_labels(labels)?;
    }
    if revision.set.status.is_some()
        && revision
            .reason
            .as_deref()
            .is_some_and(|reason| reason.trim().is_empty())
    {
        return invalid("status change reason must be non-empty when present");
    }
    validate_external_refs(&revision.external_refs.add)?;
    validate_external_refs(&revision.external_refs.remove)?;
    let scalar_change = revision.set != RevisionSet::default() || !revision.clear.is_empty();
    if !scalar_change
        && revision.relations.add.is_empty()
        && revision.relations.remove.is_empty()
        && revision.external_refs.add.is_empty()
        && revision.external_refs.remove.is_empty()
    {
        return invalid("empty Tracker revision");
    }
    let clear: HashSet<&str> = revision.clear.iter().map(String::as_str).collect();
    if clear.len() != revision.clear.len() {
        return invalid("duplicate clear field");
    }
    let set_names = revision_set_names(&revision.set);
    if clear.iter().any(|field| set_names.contains(field)) {
        return invalid("field appears in both set and clear");
    }
    Ok(())
}

fn revision_set_names(set: &RevisionSet) -> HashSet<&'static str> {
    let mut names = HashSet::new();
    if set.identifier.is_some() {
        names.insert("identifier");
    }
    if set.title.is_some() {
        names.insert("title");
    }
    if set.body.is_some() {
        names.insert("body");
    }
    if set.status.is_some() {
        names.insert("status");
    }
    if set.priority.is_some() {
        names.insert("priority");
    }
    if set.assignee.is_some() {
        names.insert("assignee");
    }
    if set.project_id.is_some() {
        names.insert("project_id");
    }
    if set.due.is_some() {
        names.insert("due");
    }
    if set.estimate_minutes.is_some() {
        names.insert("estimate_minutes");
    }
    if set.home_channel_id.is_some() {
        names.insert("home_channel_id");
    }
    if set.workflow.is_some() {
        names.insert("workflow");
    }
    if set.labels.is_some() {
        names.insert("labels");
    }
    names
}

fn validate_comment(comment: &IssueComment) -> Result<(), SdkError> {
    if comment.v != TRACKER_VERSION || comment.body.trim().is_empty() {
        return invalid("invalid Tracker comment");
    }
    validate_uuid_versions(comment.issue_id, "issue_id", &[4, 5])?;
    if let Some(request_id) = comment.request_id {
        validate_uuid_versions(request_id, "request_id", &[4, 5])?;
    }
    if let Some(import) = &comment.import {
        validate_import_provenance(import)?;
    }
    Ok(())
}

fn validate_import_provenance(import: &ImportProvenance) -> Result<(), SdkError> {
    if import.system.trim().is_empty() || import.system.len() > 64 {
        return invalid("import system must contain 1..=64 bytes");
    }
    if import.external_id.trim().is_empty() || import.external_id.len() > 256 {
        return invalid("import external_id must contain 1..=256 bytes");
    }
    if import.imported_at == 0 {
        return invalid("import imported_at must be positive");
    }
    for (name, value, limit) in [
        ("source_url", import.source_url.as_deref(), 2048),
        ("importer_version", import.importer_version.as_deref(), 128),
        ("original_author", import.original_author.as_deref(), 512),
    ] {
        if value.is_some_and(|value| value.trim().is_empty() || value.len() > limit) {
            return Err(SdkError::InvalidInput(format!(
                "import {name} must contain 1..={limit} bytes"
            )));
        }
    }
    for (name, value) in [
        ("source_created_at", import.source_created_at.as_deref()),
        ("source_updated_at", import.source_updated_at.as_deref()),
        ("source_completed_at", import.source_completed_at.as_deref()),
        ("source_cancelled_at", import.source_cancelled_at.as_deref()),
    ] {
        if value.is_some_and(|value| chrono::DateTime::parse_from_rfc3339(value).is_err()) {
            return Err(SdkError::InvalidInput(format!(
                "import {name} must be RFC 3339"
            )));
        }
    }
    if import.export_sha256.as_deref().is_some_and(|value| {
        value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        return invalid("import export_sha256 must be 64 hexadecimal characters");
    }
    if import.legacy_labels.len() > 64
        || import
            .legacy_labels
            .iter()
            .any(|value| value.trim().is_empty() || value.len() > 256)
    {
        return invalid("import legacy_labels must contain at most 64 non-empty 256-byte values");
    }
    Ok(())
}

fn validate_title(value: &str) -> Result<(), SdkError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 256 {
        return invalid("title must be 1..=256 UTF-8 bytes after trimming");
    }
    Ok(())
}

fn validate_identifier(value: &str) -> Result<(), SdkError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 64 {
        return invalid("identifier must be 1..=64 UTF-8 bytes");
    }
    Ok(())
}

fn validate_due(due: &Due) -> Result<(), SdkError> {
    let timezone = Tz::from_str(&due.timezone)
        .map_err(|_| SdkError::InvalidInput("due.timezone must be an IANA zone".into()))?;
    if due.precision == DuePrecision::Date {
        let value = i64::try_from(due.value)
            .map_err(|_| SdkError::InvalidInput("due.value is out of range".into()))?;
        let local = timezone
            .timestamp_opt(value, 0)
            .single()
            .ok_or_else(|| SdkError::InvalidInput("due.value is not a unique instant".into()))?;
        if local.hour() != 0 || local.minute() != 0 || local.second() != 0 {
            return invalid("date-precision due values must be local midnight");
        }
    }
    Ok(())
}

fn validate_labels(labels: &[String]) -> Result<BTreeSet<String>, SdkError> {
    if labels.len() > 16 {
        return invalid("labels may contain at most 16 values");
    }
    let mut normalized = BTreeSet::new();
    for label in labels {
        if label.is_empty() || label.len() > 64 || label != &label.trim().to_lowercase() {
            return invalid("labels must be normalized lowercase values of 1..=64 bytes");
        }
        if !normalized.insert(label.clone()) {
            return invalid("labels must be unique");
        }
    }
    Ok(normalized)
}

fn validate_external_refs(values: &[ExternalRef]) -> Result<(), SdkError> {
    let mut seen = BTreeSet::new();
    for value in values {
        if value.kind.trim().is_empty() || value.reference.trim().is_empty() {
            return invalid("external ref type/ref cannot be empty");
        }
        if !seen.insert(value) {
            return invalid("duplicate external ref operation");
        }
    }
    Ok(())
}

fn validate_optional_pubkey(value: Option<&str>) -> Result<(), SdkError> {
    if let Some(value) = value {
        validate_hex(value, "assignee")?;
    }
    Ok(())
}

fn validate_hex(value: &str, field: &str) -> Result<(), SdkError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return invalid(&format!("{field} must be 64-character lowercase hex"));
    }
    Ok(())
}

fn validate_uuid_v4(value: Uuid, field: &str) -> Result<(), SdkError> {
    validate_uuid_versions(value, field, &[4])
}

fn validate_uuid_versions(value: Uuid, field: &str, versions: &[usize]) -> Result<(), SdkError> {
    if !versions.contains(&value.get_version_num()) {
        return invalid(&format!("{field} has an unsupported UUID version"));
    }
    Ok(())
}

fn parse_canonical_uuid(value: &str, field: &str, versions: &[usize]) -> Result<Uuid, SdkError> {
    let parsed = Uuid::parse_str(value)
        .map_err(|_| SdkError::InvalidInput(format!("{field} must be a UUID")))?;
    validate_uuid_versions(parsed, field, versions)?;
    if parsed.to_string() != value {
        return invalid(&format!("{field} must be a canonical lowercase UUID"));
    }
    Ok(parsed)
}

fn strict_json<T: for<'de> Deserialize<'de>>(content: &str) -> Result<T, SdkError> {
    serde_json::from_str(content)
        .map_err(|error| SdkError::InvalidInput(format!("malformed Tracker JSON: {error}")))
}

fn singleton_uuid_value_tag(
    event: &Event,
    name: &str,
    versions: &[usize],
) -> Result<Uuid, SdkError> {
    let tags = tag_values(event, name);
    if tags.len() != 1 || tags[0].len() != 2 {
        return invalid(&format!("requires exactly one well-formed {name} tag"));
    }
    parse_canonical_uuid(&tags[0][1], &format!("{name} tag"), versions)
}

fn optional_uuid_value_tag(
    event: &Event,
    name: &str,
    versions: &[usize],
) -> Result<Option<Uuid>, SdkError> {
    let tags = tag_values(event, name);
    if tags.len() > 1 || tags.first().is_some_and(|tag| tag.len() != 2) {
        return invalid(&format!("{name} tag must be unique and well formed"));
    }
    tags.first()
        .map(|tag| parse_canonical_uuid(&tag[1], &format!("{name} tag"), versions))
        .transpose()
}

fn singleton_string_tag(event: &Event, name: &str) -> Result<String, SdkError> {
    let tags = tag_values(event, name);
    if tags.len() != 1 || tags[0].len() != 2 || tags[0][1].is_empty() {
        return invalid(&format!("requires exactly one non-empty {name} tag"));
    }
    Ok(tags[0][1].clone())
}

fn optional_string_tag(event: &Event, name: &str) -> Result<Option<String>, SdkError> {
    let tags = tag_values(event, name);
    if tags.len() > 1 || tags.first().is_some_and(|tag| tag.len() != 2) {
        return invalid(&format!("{name} tag must be unique and well formed"));
    }
    Ok(tags.first().map(|tag| tag[1].clone()))
}

fn require_discovery_tag(event: &Event) -> Result<(), SdkError> {
    let matches = tag_values(event, "t")
        .into_iter()
        .filter(|tag| tag.as_slice() == ["t", TRACKER_DISCOVERY_TAG])
        .count();
    if matches != 1 {
        return invalid("requires exactly one t:buzz-tracker discovery tag");
    }
    Ok(())
}

fn require_single_marked_event_tag(event: &Event, marker: &str) -> Result<String, SdkError> {
    let tags: Vec<_> = tag_values(event, "e")
        .into_iter()
        .filter(|tag| tag.len() == 4 && tag[3] == marker)
        .collect();
    if tags.len() != 1 {
        return invalid(&format!("requires exactly one e:{marker} tag"));
    }
    validate_hex(&tags[0][1], "e tag")?;
    Ok(tags[0][1].clone())
}

fn optional_single_marked_event_tag(
    event: &Event,
    marker: &str,
) -> Result<Option<String>, SdkError> {
    let tags: Vec<_> = tag_values(event, "e")
        .into_iter()
        .filter(|tag| tag.len() == 4 && tag[3] == marker)
        .collect();
    if tags.len() > 1 {
        return invalid(&format!("allows at most one e:{marker} tag"));
    }
    tags.first()
        .map(|tag| {
            validate_hex(&tag[1], "e tag")?;
            Ok(tag[1].clone())
        })
        .transpose()
}

fn validate_root_projection_tags(event: &Event, root: &IssueRoot) -> Result<(), SdkError> {
    require_exact_tag(event, "subject", &root.title)?;
    require_exact_tag(event, "status", status_str(root.status))?;
    require_exact_tag(event, "identifier", &root.identifier)?;
    require_optional_exact_tag(event, "priority", root.priority.map(priority_str))?;
    require_optional_exact_tag(
        event,
        "project",
        root.project_id.map(|value| value.to_string()).as_deref(),
    )?;
    require_optional_exact_tag(
        event,
        "home",
        root.home_channel_id
            .map(|value| value.to_string())
            .as_deref(),
    )?;
    validate_optional_estimate_tag(event, root.estimate_minutes)?;
    validate_optional_assignee_tag(event, root.assignee.as_deref())?;
    validate_optional_due_tag(event, root.due.as_ref())?;
    validate_optional_source_tag(event, root.source_event_id.as_deref())?;
    let mut labels: Vec<_> = tag_values(event, "l")
        .into_iter()
        .map(|tag| {
            if tag.len() != 2 {
                return invalid("l tags must contain exactly one value");
            }
            Ok(tag[1].clone())
        })
        .collect::<Result<_, SdkError>>()?;
    let mut expected_labels = root.labels.clone();
    labels.sort();
    expected_labels.sort();
    if labels != expected_labels {
        return invalid("l tags disagree with labels content");
    }
    let external_tags = tag_values(event, "external_ref");
    let expected_external: Vec<Vec<String>> = root
        .external_refs
        .iter()
        .map(|value| {
            vec![
                "external_ref".into(),
                value.kind.clone(),
                value.reference.clone(),
            ]
        })
        .collect();
    if sorted_tags(external_tags) != sorted_tags(expected_external) {
        return invalid("external_ref tags disagree with root content");
    }
    Ok(())
}

fn validate_revision_identity_tags(
    event: &Event,
    revision: &IssueRevision,
) -> Result<(), SdkError> {
    if tag_values(event, "e").len() != 2 {
        return invalid("revision e tags must be exactly one root and one reply");
    }
    if require_single_marked_event_tag(event, "root")? != revision.root_event
        || require_single_marked_event_tag(event, "reply")? != revision.parent_event
    {
        return invalid("revision e tags disagree with content");
    }
    require_exact_tag(event, "revision", &revision.revision.to_string())?;
    require_optional_exact_tag(event, "subject", revision.set.title.as_deref())?;
    require_optional_exact_tag(event, "status", revision.set.status.map(status_str))?;
    require_optional_exact_tag(event, "identifier", revision.set.identifier.as_deref())?;
    require_optional_exact_tag(event, "priority", revision.set.priority.map(priority_str))?;
    require_optional_exact_tag(
        event,
        "project",
        revision
            .set
            .project_id
            .map(|value| value.to_string())
            .as_deref(),
    )?;
    require_optional_exact_tag(
        event,
        "home",
        revision
            .set
            .home_channel_id
            .map(|value| value.to_string())
            .as_deref(),
    )?;
    validate_optional_estimate_tag(event, revision.set.estimate_minutes)?;
    validate_optional_assignee_tag(event, revision.set.assignee.as_deref())?;
    validate_optional_due_tag(event, revision.set.due.as_ref())?;
    let expected_labels = revision.set.labels.as_deref().unwrap_or_default();
    let labels = tag_values(event, "l");
    let expected_label_tags: Vec<Vec<String>> = expected_labels
        .iter()
        .map(|value| vec!["l".into(), value.clone()])
        .collect();
    if sorted_tags(labels) != sorted_tags(expected_label_tags) {
        return invalid("l tags disagree with revision content");
    }
    let expected_clear: Vec<Vec<String>> = revision
        .clear
        .iter()
        .map(|value| vec!["clear".into(), value.clone()])
        .collect();
    if sorted_tags(tag_values(event, "clear")) != sorted_tags(expected_clear) {
        return invalid("clear tags disagree with revision content");
    }
    let mut expected_relations = Vec::new();
    for (operation, relations) in [
        ("add", &revision.relations.add),
        ("remove", &revision.relations.remove),
    ] {
        expected_relations.extend(relations.iter().map(|value| {
            vec![
                "relation".into(),
                operation.into(),
                relation_str(value.kind).into(),
                value.target_root_event.clone(),
            ]
        }));
    }
    if sorted_tags(tag_values(event, "relation")) != sorted_tags(expected_relations) {
        return invalid("relation tags disagree with revision content");
    }
    let mut expected_external = Vec::new();
    for (operation, refs) in [
        ("add", &revision.external_refs.add),
        ("remove", &revision.external_refs.remove),
    ] {
        expected_external.extend(refs.iter().map(|value| {
            vec![
                "external_ref".into(),
                operation.into(),
                value.kind.clone(),
                value.reference.clone(),
            ]
        }));
    }
    if sorted_tags(tag_values(event, "external_ref")) != sorted_tags(expected_external) {
        return invalid("external_ref tags disagree with revision content");
    }
    Ok(())
}

fn validate_allowed_tags(event: &Event, allowed: &[&str]) -> Result<(), SdkError> {
    for tag in event.tags.iter() {
        let Some(name) = tag.as_slice().first() else {
            return invalid("empty Tracker tag");
        };
        if !allowed.contains(&name.as_str()) {
            return invalid(&format!("unknown Tracker tag {name:?}"));
        }
    }
    Ok(())
}

fn require_optional_exact_tag(
    event: &Event,
    name: &str,
    value: Option<&str>,
) -> Result<(), SdkError> {
    let tags = tag_values(event, name);
    match value {
        Some(value) if tags.len() == 1 && tags[0].as_slice() == [name, value] => Ok(()),
        None if tags.is_empty() => Ok(()),
        _ => invalid(&format!("{name} tag disagrees with content")),
    }
}

fn validate_optional_estimate_tag(event: &Event, estimate: Option<u32>) -> Result<(), SdkError> {
    let tags = tag_values(event, "estimate");
    match estimate {
        Some(value)
            if tags.len() == 1
                && tags[0].len() == 3
                && tags[0][1] == value.to_string()
                && tags[0][2] == "minutes" =>
        {
            Ok(())
        }
        None if tags.is_empty() => Ok(()),
        _ => invalid("estimate tag disagrees with content"),
    }
}

fn validate_optional_assignee_tag(event: &Event, assignee: Option<&str>) -> Result<(), SdkError> {
    let tags = tag_values(event, "p");
    match assignee {
        Some(value)
            if tags.len() == 1
                && tags[0].len() == 4
                && tags[0][1] == value
                && tags[0][3] == "assignee" =>
        {
            Ok(())
        }
        None if tags.is_empty() => Ok(()),
        _ => invalid("p:assignee tag disagrees with content"),
    }
}

fn validate_optional_due_tag(event: &Event, due: Option<&Due>) -> Result<(), SdkError> {
    let tags = tag_values(event, "due");
    match due {
        Some(value)
            if tags.len() == 1
                && tags[0].len() == 4
                && tags[0][1] == value.value.to_string()
                && tags[0][2] == due_precision_str(value.precision)
                && tags[0][3] == value.timezone =>
        {
            Ok(())
        }
        None if tags.is_empty() => Ok(()),
        _ => invalid("due tag disagrees with content"),
    }
}

fn validate_optional_source_tag(event: &Event, source: Option<&str>) -> Result<(), SdkError> {
    let tags = tag_values(event, "e");
    match source {
        Some(value)
            if tags.len() == 1
                && tags[0].len() == 4
                && tags[0][1] == value
                && tags[0][3] == "source" =>
        {
            Ok(())
        }
        None if tags.is_empty() => Ok(()),
        _ => invalid("e:source tag disagrees with content"),
    }
}

fn sorted_tags(mut tags: Vec<Vec<String>>) -> Vec<Vec<String>> {
    tags.sort();
    tags
}

fn require_exact_tag(event: &Event, name: &str, value: &str) -> Result<(), SdkError> {
    let tags = tag_values(event, name);
    if tags.len() != 1 || tags[0].as_slice() != [name, value] {
        return invalid(&format!("{name} tag disagrees with content"));
    }
    Ok(())
}

fn tag_values(event: &Event, name: &str) -> Vec<Vec<String>> {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .filter(|tag| tag.first().is_some_and(|value| value == name))
        .collect()
}

fn status_str(status: IssueStatus) -> &'static str {
    match status {
        IssueStatus::Triage => "triage",
        IssueStatus::Backlog => "backlog",
        IssueStatus::Todo => "todo",
        IssueStatus::InProgress => "in_progress",
        IssueStatus::InReview => "in_review",
        IssueStatus::Done => "done",
        IssueStatus::Cancelled => "cancelled",
    }
}

fn priority_str(priority: Priority) -> &'static str {
    match priority {
        Priority::P0 => "P0",
        Priority::P1 => "P1",
        Priority::P2 => "P2",
        Priority::P3 => "P3",
    }
}

fn due_precision_str(precision: DuePrecision) -> &'static str {
    match precision {
        DuePrecision::Instant => "instant",
        DuePrecision::Date => "date",
    }
}

fn relation_str(relation: RelationType) -> &'static str {
    match relation {
        RelationType::Blocks => "blocks",
        RelationType::DuplicateOf => "duplicate_of",
        RelationType::Parent => "parent",
    }
}

fn invalid<T>(message: &str) -> Result<T, SdkError> {
    Err(SdkError::InvalidInput(message.into()))
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    use super::*;

    const MEMBER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const MAINTAINER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const OTHER: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    fn auth() -> TrackerAuthorization {
        TrackerAuthorization::new([
            (MEMBER.into(), TrackerRole::Member),
            (MAINTAINER.into(), TrackerRole::Maintainer),
        ])
    }

    fn root(stream: Uuid, issue: Uuid, id: &str, created_at: u64, author: &str) -> TrackerEvent {
        TrackerEvent {
            event_id: id.into(),
            author_pubkey: author.into(),
            stream_id: stream,
            issue_id: issue,
            created_at,
            payload: TrackerPayload::Root(IssueRoot {
                v: TRACKER_VERSION,
                op: "create".into(),
                issue_id: issue,
                identifier: "BT-1".into(),
                title: "Root title".into(),
                body: "body".into(),
                status: IssueStatus::Triage,
                priority: None,
                assignee: None,
                project_id: None,
                due: None,
                estimate_minutes: None,
                home_channel_id: None,
                source_event_id: None,
                workflow: "default-v1".into(),
                labels: vec!["area:tracker".into()],
                external_refs: Vec::new(),
                request_id: None,
                import: None,
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn revision(
        stream: Uuid,
        issue: Uuid,
        id: &str,
        parent: &str,
        number: u64,
        created_at: u64,
        author: &str,
        set: RevisionSet,
    ) -> TrackerEvent {
        TrackerEvent {
            event_id: id.into(),
            author_pubkey: author.into(),
            stream_id: stream,
            issue_id: issue,
            created_at,
            payload: TrackerPayload::Revision(IssueRevision {
                v: TRACKER_VERSION,
                op: "revise".into(),
                issue_id: issue,
                root_event: "root".into(),
                parent_event: parent.into(),
                revision: number,
                set,
                clear: Vec::new(),
                relations: SetOperations::default(),
                external_refs: SetOperations::default(),
                reason: Some("fixture transition".into()),
                request_id: None,
                import: None,
            }),
        }
    }

    fn current(events: Vec<TrackerEvent>) -> (IssueProjection, Vec<FoldDiagnostic>) {
        let stream = events[0].stream_id;
        let issue = events[0].issue_id;
        let mut folded = fold_tracker_events(events, &auth());
        (
            folded
                .issues
                .remove(&(stream, issue))
                .expect("fixture issue"),
            folded.diagnostics,
        )
    }

    #[test]
    fn import_provenance_omits_additive_fields_when_empty() {
        let provenance = ImportProvenance {
            system: "linear".into(),
            external_id: "LIN-1".into(),
            source_url: None,
            imported_at: 1,
            source_created_at: None,
            source_updated_at: None,
            source_completed_at: None,
            source_cancelled_at: None,
            archived: None,
            estimate_points: None,
            export_sha256: None,
            importer_version: None,
            original_author: None,
            legacy_labels: vec![],
        };

        assert_eq!(
            serde_json::to_value(provenance).unwrap(),
            serde_json::json!({
                "system": "linear",
                "external_id": "LIN-1",
                "source_url": null,
                "imported_at": 1,
            })
        );
    }

    #[test]
    fn gate_01_root_only_issue() {
        let stream = Uuid::new_v4();
        let issue = Uuid::new_v4();
        let (state, diagnostics) = current(vec![root(stream, issue, "root", 1, MEMBER)]);
        assert_eq!(state.tip_event, "root");
        assert_eq!(state.tip_created_at, 1);
        assert_eq!(state.revision, 0);
        assert_eq!(state.entered_status_at, 1);
        assert_eq!(state.status, IssueStatus::Triage);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn gate_02_linear_revision_chain() {
        let stream = Uuid::new_v4();
        let issue = Uuid::new_v4();
        let one = revision(
            stream,
            issue,
            "one",
            "root",
            1,
            2,
            MEMBER,
            RevisionSet {
                title: Some("changed".into()),
                ..RevisionSet::default()
            },
        );
        let two = revision(
            stream,
            issue,
            "two",
            "one",
            2,
            3,
            MEMBER,
            RevisionSet {
                status: Some(IssueStatus::InProgress),
                ..RevisionSet::default()
            },
        );
        let folded = fold_tracker_events(
            vec![root(stream, issue, "root", 1, MEMBER), one, two],
            &auth(),
        );
        let state = &folded.issues[&(stream, issue)];
        assert_eq!(state.tip_event, "two");
        assert_eq!(state.tip_created_at, 3);
        assert_eq!(state.revision, 2);
        assert_eq!(state.entered_status_at, 3);
        assert_eq!(state.title, "changed");
        assert_eq!(state.status, IssueStatus::InProgress);
        let activity = &folded.activity[&(stream, issue)];
        assert_eq!(
            activity
                .iter()
                .map(|entry| entry.event_id.as_str())
                .collect::<Vec<_>>(),
            ["root", "one", "two"]
        );
        assert!(activity[0].before.is_none());
        assert_eq!(activity[0].revision, 0);
        assert_eq!(activity[1].after.entered_status_at, 1);
        assert_eq!(activity[2].revision, 2);
        assert_eq!(
            activity[2].before.as_ref().map(|before| before.status),
            Some(IssueStatus::Triage)
        );
        assert_eq!(activity[2].after.status, IssueStatus::InProgress);
        assert!(folded.diagnostics.is_empty());
    }

    #[test]
    fn gate_03_duplicate_and_reordered_delivery_is_deterministic() {
        let stream = Uuid::new_v4();
        let issue = Uuid::new_v4();
        let root_event = root(stream, issue, "root", 1, MEMBER);
        let one = revision(
            stream,
            issue,
            "one",
            "root",
            1,
            2,
            MEMBER,
            RevisionSet {
                title: Some("changed".into()),
                ..RevisionSet::default()
            },
        );
        let first = current(vec![root_event.clone(), one.clone()]).0;
        let second = current(vec![one.clone(), root_event, one]).0;
        assert_eq!(first, second);
    }

    #[test]
    fn true_fork_flags_loser_with_canonical_tip() {
        let stream = Uuid::new_v4();
        let issue = Uuid::new_v4();
        let loser = revision(
            stream,
            issue,
            "loser",
            "root",
            1,
            2,
            MEMBER,
            RevisionSet {
                title: Some("losing branch".into()),
                ..RevisionSet::default()
            },
        );
        let winner = revision(
            stream,
            issue,
            "winner",
            "root",
            1,
            3,
            MEMBER,
            RevisionSet {
                title: Some("winning branch".into()),
                ..RevisionSet::default()
            },
        );
        let (state, diagnostics) =
            current(vec![root(stream, issue, "root", 1, MEMBER), loser, winner]);

        assert_eq!(state.tip_event, "winner");
        assert_eq!(
            diagnostics,
            [FoldDiagnostic::LosingBranch {
                issue_id: issue,
                event_id: "loser".into(),
                canonical_tip: "winner".into(),
            }]
        );
    }

    #[test]
    fn gate_04_stale_branch_uses_recency_not_revision_depth() {
        let stream = Uuid::new_v4();
        let issue = Uuid::new_v4();
        let deep_one = revision(
            stream,
            issue,
            "deep-one",
            "root",
            1,
            10,
            MEMBER,
            RevisionSet {
                assignee: Some(MEMBER.into()),
                ..RevisionSet::default()
            },
        );
        let deep_two = revision(
            stream,
            issue,
            "deep-two",
            "deep-one",
            2,
            11,
            MEMBER,
            RevisionSet {
                title: Some("deep stale".into()),
                ..RevisionSet::default()
            },
        );
        let recent = revision(
            stream,
            issue,
            "recent",
            "root",
            1,
            20,
            MEMBER,
            RevisionSet {
                assignee: Some(MAINTAINER.into()),
                ..RevisionSet::default()
            },
        );
        let (state, diagnostics) = current(vec![
            root(stream, issue, "root", 1, MEMBER),
            deep_one,
            deep_two,
            recent,
        ]);
        assert_eq!(state.tip_event, "recent");
        assert_eq!(state.assignee.as_deref(), Some(MAINTAINER));
        assert_eq!(
            diagnostics,
            [
                FoldDiagnostic::LosingBranch {
                    issue_id: issue,
                    event_id: "deep-one".into(),
                    canonical_tip: "recent".into(),
                },
                FoldDiagnostic::LosingBranch {
                    issue_id: issue,
                    event_id: "deep-two".into(),
                    canonical_tip: "recent".into(),
                },
            ]
        );
    }

    #[test]
    fn gate_05_malformed_parent_and_revision_are_excluded() {
        let stream = Uuid::new_v4();
        let issue = Uuid::new_v4();
        let bad_parent = revision(
            stream,
            issue,
            "bad-parent",
            "missing",
            1,
            2,
            MEMBER,
            RevisionSet {
                title: Some("bad".into()),
                ..RevisionSet::default()
            },
        );
        let bad_number = revision(
            stream,
            issue,
            "bad-number",
            "root",
            2,
            3,
            MEMBER,
            RevisionSet {
                title: Some("bad".into()),
                ..RevisionSet::default()
            },
        );
        let (state, diagnostics) = current(vec![
            root(stream, issue, "root", 1, MEMBER),
            bad_parent,
            bad_number,
        ]);
        assert_eq!(state.tip_event, "root");
        assert_eq!(diagnostics.len(), 2);
    }

    #[test]
    fn gate_06_status_change_reason_is_optional_but_not_blank() {
        let stream = Uuid::new_v4();
        let issue = Uuid::new_v4();
        let mut absent = revision(
            stream,
            issue,
            "absent",
            "root",
            1,
            2,
            MEMBER,
            RevisionSet {
                status: Some(IssueStatus::Todo),
                ..RevisionSet::default()
            },
        );
        let TrackerPayload::Revision(payload) = &mut absent.payload else {
            panic!("fixture revision")
        };
        payload.reason = None;
        let (state, diagnostics) =
            current(vec![root(stream, issue, "root", 1, MEMBER), absent.clone()]);
        assert_eq!(state.status, IssueStatus::Todo);
        assert!(diagnostics.is_empty());

        let TrackerPayload::Revision(payload) = &mut absent.payload else {
            panic!("fixture revision")
        };
        payload.reason = Some("   ".into());
        let (state, diagnostics) = current(vec![root(stream, issue, "root", 1, MEMBER), absent]);
        assert_eq!(state.status, IssueStatus::Triage);
        assert_eq!(diagnostics.len(), 1);

        let builder_payload = IssueRevision {
            v: TRACKER_VERSION,
            op: "revise".into(),
            issue_id: issue,
            root_event: MEMBER.into(),
            parent_event: MEMBER.into(),
            revision: 1,
            set: RevisionSet {
                status: Some(IssueStatus::Todo),
                ..RevisionSet::default()
            },
            clear: Vec::new(),
            relations: SetOperations::default(),
            external_refs: SetOperations::default(),
            reason: None,
            request_id: None,
            import: None,
        };
        assert!(build_tracker_revision(stream, &builder_payload).is_ok());
        assert!(build_tracker_revision(
            stream,
            &IssueRevision {
                reason: Some("\t".into()),
                ..builder_payload
            }
        )
        .is_err());
    }

    #[test]
    fn gate_07_relation_add_remove_and_self_edge_rules() {
        let stream = Uuid::new_v4();
        let issue = Uuid::new_v4();
        let target = Uuid::new_v4();
        let relation = Relation {
            kind: RelationType::Blocks,
            target_issue_id: target,
            target_root_event: OTHER.into(),
        };
        let mut add = revision(
            stream,
            issue,
            "add",
            "root",
            1,
            2,
            MEMBER,
            RevisionSet {
                title: Some("changed".into()),
                ..RevisionSet::default()
            },
        );
        let TrackerPayload::Revision(payload) = &mut add.payload else {
            panic!("fixture revision")
        };
        payload.relations.add.push(relation.clone());
        let mut remove = revision(
            stream,
            issue,
            "remove",
            "add",
            2,
            3,
            MEMBER,
            RevisionSet {
                body: Some("changed".into()),
                ..RevisionSet::default()
            },
        );
        let TrackerPayload::Revision(payload) = &mut remove.payload else {
            panic!("fixture revision")
        };
        payload.relations.remove.push(relation);
        let (state, _) = current(vec![root(stream, issue, "root", 1, MEMBER), add, remove]);
        assert!(state.relations.is_empty());

        let mut self_edge = revision(
            stream,
            issue,
            "self-edge",
            "root",
            1,
            4,
            MEMBER,
            RevisionSet {
                title: Some("invalid self edge".into()),
                ..RevisionSet::default()
            },
        );
        let TrackerPayload::Revision(payload) = &mut self_edge.payload else {
            panic!("fixture revision")
        };
        payload.relations.add.push(Relation {
            kind: RelationType::Parent,
            target_issue_id: issue,
            target_root_event: "root".into(),
        });
        let (state, diagnostics) = current(vec![root(stream, issue, "root", 1, MEMBER), self_edge]);
        assert!(state.relations.is_empty());
        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            FoldDiagnostic::InvalidEvent { reason, .. } if reason.contains("self relation")
        )));
    }

    #[test]
    fn invalid_parent_cycle_is_removed_while_blocks_cycle_is_surfaced() {
        let stream = Uuid::new_v4();
        let left_id = Uuid::new_v4();
        let right_id = Uuid::new_v4();
        let mut left = revision(
            stream,
            left_id,
            "left-revision",
            "left-root",
            1,
            2,
            MEMBER,
            RevisionSet {
                title: Some("left".into()),
                ..RevisionSet::default()
            },
        );
        let TrackerPayload::Revision(payload) = &mut left.payload else {
            panic!("fixture revision")
        };
        payload.root_event = "left-root".into();
        payload.relations.add.extend([
            Relation {
                kind: RelationType::Parent,
                target_issue_id: right_id,
                target_root_event: "right-root".into(),
            },
            Relation {
                kind: RelationType::Blocks,
                target_issue_id: right_id,
                target_root_event: "right-root".into(),
            },
        ]);
        let mut right = revision(
            stream,
            right_id,
            "right-revision",
            "right-root",
            1,
            2,
            MEMBER,
            RevisionSet {
                title: Some("right".into()),
                ..RevisionSet::default()
            },
        );
        let TrackerPayload::Revision(payload) = &mut right.payload else {
            panic!("fixture revision")
        };
        payload.root_event = "right-root".into();
        payload.relations.add.extend([
            Relation {
                kind: RelationType::Parent,
                target_issue_id: left_id,
                target_root_event: "left-root".into(),
            },
            Relation {
                kind: RelationType::Blocks,
                target_issue_id: left_id,
                target_root_event: "left-root".into(),
            },
        ]);
        let folded = fold_tracker_events(
            [
                root(stream, left_id, "left-root", 1, MEMBER),
                root(stream, right_id, "right-root", 1, MEMBER),
                left,
                right,
            ],
            &auth(),
        );
        assert!(folded.issues.values().all(|issue| issue
            .relations
            .iter()
            .all(|relation| relation.kind != RelationType::Parent)));
        assert!(folded
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic, FoldDiagnostic::BlocksCycle { .. })));
    }

    #[test]
    fn gate_08_duplicate_root_earliest_order_wins() {
        let stream = Uuid::new_v4();
        let issue = Uuid::new_v4();
        let late = root(stream, issue, "z-root", 2, MEMBER);
        let early = root(stream, issue, "a-root", 1, MEMBER);
        let (state, diagnostics) = current(vec![late, early]);
        assert_eq!(state.root_event, "a-root");
        assert!(matches!(
            diagnostics[0],
            FoldDiagnostic::RootCollision { .. }
        ));
    }

    #[test]
    fn gate_09_comments_never_change_state_tip() {
        let stream = Uuid::new_v4();
        let issue = Uuid::new_v4();
        let comment = TrackerEvent {
            event_id: "comment".into(),
            author_pubkey: MEMBER.into(),
            stream_id: stream,
            issue_id: issue,
            created_at: 99,
            payload: TrackerPayload::Comment(TrackerComment {
                content: IssueComment {
                    v: 1,
                    issue_id: issue,
                    body: "hello".into(),
                    request_id: None,
                    import: None,
                },
                root_event: "root".into(),
                parent_event: None,
            }),
        };
        let events = vec![root(stream, issue, "root", 1, MEMBER), comment];
        let (state, _) = current(events.clone());
        assert_eq!(state.tip_event, "root");
        let folded = fold_tracker_events(events, &auth());
        assert_eq!(folded.comments[&(stream, issue)][0].body, "hello");
    }

    #[test]
    fn gate_10_workspace_roles_are_enforced_in_fold() {
        let stream = Uuid::new_v4();
        let issue = Uuid::new_v4();
        let outsider = revision(
            stream,
            issue,
            "outsider",
            "root",
            1,
            2,
            OTHER,
            RevisionSet {
                title: Some("unauthorized".into()),
                ..RevisionSet::default()
            },
        );
        let terminal = revision(
            stream,
            issue,
            "terminal",
            "root",
            1,
            3,
            MEMBER,
            RevisionSet {
                status: Some(IssueStatus::Done),
                ..RevisionSet::default()
            },
        );
        let (state, diagnostics) = current(vec![
            root(stream, issue, "root", 1, MEMBER),
            outsider,
            terminal,
        ]);
        assert_eq!(state.tip_event, "root");
        assert_eq!(diagnostics.len(), 2);
    }

    #[test]
    fn concurrent_external_ref_adds_both_survive() {
        let stream = Uuid::new_v4();
        let issue = Uuid::new_v4();
        let mut left = revision(
            stream,
            issue,
            "left",
            "root",
            1,
            2,
            MEMBER,
            RevisionSet {
                title: Some("left".into()),
                ..RevisionSet::default()
            },
        );
        let mut right = revision(
            stream,
            issue,
            "right",
            "root",
            1,
            3,
            MEMBER,
            RevisionSet {
                title: Some("right".into()),
                ..RevisionSet::default()
            },
        );
        let TrackerPayload::Revision(payload) = &mut left.payload else {
            panic!("fixture revision")
        };
        payload.external_refs.add.push(ExternalRef {
            kind: "pr".into(),
            reference: "1".into(),
        });
        let TrackerPayload::Revision(payload) = &mut right.payload else {
            panic!("fixture revision")
        };
        payload.external_refs.add.push(ExternalRef {
            kind: "pr".into(),
            reference: "2".into(),
        });
        let (state, _) = current(vec![root(stream, issue, "root", 1, MEMBER), left, right]);
        assert_eq!(state.external_refs.len(), 2);
    }

    #[test]
    fn identifier_collision_is_surfaced_with_earlier_root_canonical() {
        let stream = Uuid::new_v4();
        let first_issue = Uuid::new_v4();
        let second_issue = Uuid::new_v4();
        let folded = fold_tracker_events(
            [
                root(stream, second_issue, "second-root", 2, MEMBER),
                root(stream, first_issue, "first-root", 1, MEMBER),
            ],
            &auth(),
        );
        assert!(folded
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic,
            FoldDiagnostic::IdentifierCollision { canonical, conflicting, .. }
                if *canonical == first_issue && *conflicting == second_issue)));
    }

    #[test]
    fn clock_skew_over_300_seconds_is_rejected() {
        let keys = Keys::generate();
        let stream = Uuid::new_v4();
        let issue = Uuid::new_v4();
        let content = IssueRoot {
            v: 1,
            op: "create".into(),
            issue_id: issue,
            identifier: "BT-1".into(),
            title: "title".into(),
            body: "body".into(),
            status: IssueStatus::Triage,
            priority: None,
            assignee: None,
            project_id: None,
            due: None,
            estimate_minutes: None,
            home_channel_id: None,
            source_event_id: None,
            workflow: "default-v1".into(),
            labels: vec!["area:tracker".into()],
            external_refs: Vec::new(),
            request_id: None,
            import: None,
        };
        let tags = vec![
            Tag::parse(["h", &stream.to_string()]).expect("h tag"),
            Tag::parse(["d", &issue.to_string()]).expect("d tag"),
            Tag::parse(["t", TRACKER_DISCOVERY_TAG]).expect("t tag"),
            Tag::parse(["subject", "title"]).expect("subject tag"),
            Tag::parse(["status", "triage"]).expect("status tag"),
            Tag::parse(["identifier", "BT-1"]).expect("identifier tag"),
            Tag::parse(["l", "area:tracker"]).expect("label tag"),
        ];
        let event = EventBuilder::new(
            Kind::Custom(KIND_TRACKER_ISSUE as u16),
            serde_json::to_string(&content).expect("root JSON"),
        )
        .tags(tags)
        .custom_created_at(Timestamp::from(1_301))
        .sign_with_keys(&keys)
        .expect("signed event");
        assert!(parse_tracker_event(&event, 1_301).is_ok());
        assert!(parse_tracker_event(&event, 1_000).is_err());
    }

    #[test]
    fn strict_parser_rejects_unknown_tags_and_projection_mismatch() {
        let keys = Keys::generate();
        let stream = Uuid::new_v4();
        let issue = Uuid::new_v4();
        let content = IssueRoot {
            v: 1,
            op: "create".into(),
            issue_id: issue,
            identifier: "BT-1".into(),
            title: "title".into(),
            body: "body".into(),
            status: IssueStatus::Triage,
            priority: None,
            assignee: None,
            project_id: None,
            due: None,
            estimate_minutes: None,
            home_channel_id: None,
            source_event_id: None,
            workflow: "default-v1".into(),
            labels: Vec::new(),
            external_refs: Vec::new(),
            request_id: None,
            import: None,
        };
        let base_tags = || {
            vec![
                Tag::parse(["h", &stream.to_string()]).expect("h tag"),
                Tag::parse(["d", &issue.to_string()]).expect("d tag"),
                Tag::parse(["t", TRACKER_DISCOVERY_TAG]).expect("t tag"),
                Tag::parse(["subject", "title"]).expect("subject tag"),
                Tag::parse(["status", "triage"]).expect("status tag"),
                Tag::parse(["identifier", "BT-1"]).expect("identifier tag"),
            ]
        };
        let json = serde_json::to_string(&content).expect("root JSON");
        let mut unknown_tags = base_tags();
        unknown_tags.push(Tag::parse(["x-unknown", "value"]).expect("unknown tag"));
        let unknown = EventBuilder::new(Kind::Custom(KIND_TRACKER_ISSUE as u16), json.clone())
            .tags(unknown_tags)
            .custom_created_at(Timestamp::from(100))
            .sign_with_keys(&keys)
            .expect("signed event");
        assert!(parse_tracker_event(&unknown, 100).is_err());

        let mut mismatch_tags = base_tags();
        mismatch_tags.retain(|tag| tag.as_slice().first().map(String::as_str) != Some("status"));
        mismatch_tags.push(Tag::parse(["status", "todo"]).expect("status tag"));
        let mismatch = EventBuilder::new(Kind::Custom(KIND_TRACKER_ISSUE as u16), json)
            .tags(mismatch_tags)
            .custom_created_at(Timestamp::from(100))
            .sign_with_keys(&keys)
            .expect("signed event");
        assert!(parse_tracker_event(&mismatch, 100).is_err());
    }

    #[test]
    fn inverse_relation_writes_are_canonicalized() {
        let source = Uuid::new_v4();
        let target = Uuid::new_v4();
        let write = canonicalize_relation(
            source,
            "source-root".into(),
            RelationDirection::BlockedBy,
            target,
            "target-root".into(),
        );
        assert_eq!(write.source_issue_id, target);
        assert_eq!(write.relation.kind, RelationType::Blocks);
        assert_eq!(write.relation.target_issue_id, source);
    }

    #[test]
    fn public_root_builder_round_trips_with_managed_auth_tag() {
        let keys = Keys::generate();
        let assignee = Keys::generate();
        let stream = Uuid::new_v4();
        let issue = Uuid::new_v4();
        let request = Uuid::new_v4();
        let root = IssueRoot {
            v: TRACKER_VERSION,
            op: "create".into(),
            issue_id: issue,
            identifier: "SC-1".into(),
            title: "Builder round trip".into(),
            body: "body".into(),
            status: IssueStatus::Todo,
            priority: Some(Priority::P1),
            assignee: Some(assignee.public_key().to_hex()),
            project_id: None,
            due: None,
            estimate_minutes: Some(90),
            home_channel_id: Some(Uuid::new_v4()),
            source_event_id: None,
            workflow: "default-v1".into(),
            labels: vec!["area:tracker".into()],
            external_refs: vec![ExternalRef {
                kind: "linear".into(),
                reference: "SMI-1".into(),
            }],
            request_id: Some(request),
            import: None,
        };
        let auth = Tag::parse(["auth", &"1".repeat(64), "", &"2".repeat(128)]).expect("auth tag");
        let event = build_tracker_issue(stream, &root)
            .expect("builder")
            .tag(auth)
            .custom_created_at(Timestamp::from(100))
            .sign_with_keys(&keys)
            .expect("signed event");
        let parsed = parse_tracker_event(&event, 100).expect("strict parse");
        assert_eq!(parsed.issue_id, issue);
        assert!(matches!(parsed.payload, TrackerPayload::Root(value) if value == root));
    }

    #[test]
    fn public_revision_builder_round_trips_with_assignee() {
        let keys = Keys::generate();
        let assignee = keys.public_key().to_hex();
        let stream = Uuid::new_v4();
        let issue = Uuid::new_v4();
        let revision = IssueRevision {
            v: TRACKER_VERSION,
            op: "revise".into(),
            issue_id: issue,
            root_event: "1".repeat(64),
            parent_event: "2".repeat(64),
            revision: 1,
            set: RevisionSet {
                assignee: Some(assignee),
                status: Some(IssueStatus::Todo),
                ..RevisionSet::default()
            },
            clear: Vec::new(),
            relations: SetOperations::default(),
            external_refs: SetOperations {
                add: vec![ExternalRef {
                    kind: "task".into(),
                    reference: Uuid::new_v4().to_string(),
                }],
                remove: Vec::new(),
            },
            reason: Some("dispatched for execution".into()),
            request_id: Some(Uuid::new_v4()),
            import: None,
        };
        let event = build_tracker_revision(stream, &revision)
            .expect("builder")
            .custom_created_at(Timestamp::from(100))
            .sign_with_keys(&keys)
            .expect("signed event");
        let parsed = parse_tracker_event(&event, 100).expect("strict parse");
        assert!(matches!(parsed.payload, TrackerPayload::Revision(value) if value == revision));
    }

    #[test]
    fn deterministic_import_uuid_v5_round_trips() {
        let keys = Keys::generate();
        let stream = Uuid::new_v4();
        let issue = Uuid::parse_str("6ba7b812-9dad-51d1-80b4-00c04fd430c8").unwrap();
        let request = Uuid::parse_str("6ba7b813-9dad-51d1-80b4-00c04fd430c8").unwrap();
        let mut root = match root(stream, Uuid::new_v4(), "root", 1, MEMBER).payload {
            TrackerPayload::Root(root) => root,
            _ => unreachable!(),
        };
        root.issue_id = issue;
        root.request_id = Some(request);
        let event = build_tracker_issue(stream, &root)
            .expect("UUIDv5 builder")
            .custom_created_at(Timestamp::from(100))
            .sign_with_keys(&keys)
            .expect("signed event");
        assert_eq!(
            parse_tracker_event(&event, 100).expect("parse").issue_id,
            issue
        );
    }

    #[test]
    fn project_head_and_activity_round_trip() {
        let keys = Keys::generate();
        let workspace = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let tags = vec![
            Tag::parse(["d", "tracker-core"]).unwrap(),
            Tag::parse(["name", "Tracker Core"]).unwrap(),
            Tag::parse(["buzz-project-id", &project_id.to_string()]).unwrap(),
            Tag::parse(["buzz-workspace", &workspace.to_string()]).unwrap(),
            Tag::parse(["buzz-status", "triage"]).unwrap(),
            Tag::parse(["buzz-status", "done"]).unwrap(),
        ];
        let head = crate::build_project_with_tags("", tags)
            .unwrap()
            .custom_created_at(Timestamp::from(100))
            .sign_with_keys(&keys)
            .unwrap();
        let project = parse_tracker_project(&head).expect("project parse");
        assert_eq!(project.project_id, project_id);
        assert_eq!(project.statuses, [IssueStatus::Triage, IssueStatus::Done]);

        let activity = ProjectActivity {
            v: TRACKER_VERSION,
            op: ProjectActivityOperation::Create,
            project_id,
            request_id: Uuid::new_v4(),
            before: None,
            after: TrackerProjectSnapshot::from(&project),
        };
        let event =
            build_tracker_project_activity(workspace, &activity, &keys.public_key().to_hex())
                .unwrap()
                .custom_created_at(Timestamp::from(101))
                .sign_with_keys(&keys)
                .unwrap();
        let parsed = parse_tracker_project_activity(&event, 101).expect("activity parse");
        assert_eq!(parsed.activity, activity);
    }

    fn tracker_project_head(
        keys: &Keys,
        workspace: Uuid,
        project_id: Uuid,
        slug: &str,
        created_at: u64,
    ) -> Event {
        crate::build_project_with_tags(
            "",
            vec![
                Tag::parse(["d", slug]).unwrap(),
                Tag::parse(["name", slug]).unwrap(),
                Tag::parse(["buzz-project-id", &project_id.to_string()]).unwrap(),
                Tag::parse(["buzz-workspace", &workspace.to_string()]).unwrap(),
            ],
        )
        .unwrap()
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .unwrap()
    }

    #[test]
    fn project_fold_preserves_unambiguous_heads() {
        let keys = Keys::generate();
        let workspace = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let head = tracker_project_head(&keys, workspace, project_id, "dispatch", 100);
        let projection =
            fold_tracker_projects_at([head], workspace, &keys.public_key().to_hex(), 100);
        assert_eq!(projection.projects[&project_id].slug, "dispatch");
        assert!(projection.diagnostics.is_empty());
    }

    #[test]
    fn project_fold_case_folds_slugs_and_picks_newest_canonical() {
        let keys = Keys::generate();
        let workspace = Uuid::new_v4();
        let older_id = Uuid::parse_str("ffffffff-ffff-4fff-8fff-ffffffffffff").unwrap();
        let middle_id = Uuid::parse_str("eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee").unwrap();
        let newer_id = Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap();
        let older = tracker_project_head(&keys, workspace, older_id, "Foo", 100);
        let middle = tracker_project_head(&keys, workspace, middle_id, "FOO", 101);
        let newer = tracker_project_head(&keys, workspace, newer_id, "foo", 101);
        let canonical_id = if newer.id.to_hex() > middle.id.to_hex() {
            newer_id
        } else {
            middle_id
        };
        let projection = fold_tracker_projects_at(
            [newer, older, middle],
            workspace,
            &keys.public_key().to_hex(),
            101,
        );
        assert_eq!(projection.projects.len(), 3);
        let duplicate_slugs: Vec<_> = projection
            .diagnostics
            .iter()
            .filter_map(|diagnostic| match diagnostic {
                ProjectDiagnostic::DuplicateSlug {
                    canonical,
                    conflicting,
                    ..
                } => Some((*canonical, *conflicting)),
                _ => None,
            })
            .collect();
        assert_eq!(duplicate_slugs.len(), 2);
        assert!(duplicate_slugs
            .iter()
            .all(|(canonical, _)| *canonical == canonical_id));
        assert!(duplicate_slugs
            .iter()
            .any(|(_, conflicting)| *conflicting == older_id));
    }

    #[test]
    fn project_fold_distinguishes_invalid_managed_and_unmanaged_heads() {
        let keys = Keys::generate();
        let workspace = Uuid::new_v4();
        let invalid = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags([
                Tag::parse(["d", "invalid"]).unwrap(),
                Tag::parse(["buzz-project-id", "not-a-uuid"]).unwrap(),
                Tag::parse(["buzz-workspace", &workspace.to_string()]).unwrap(),
            ])
            .custom_created_at(Timestamp::from(100))
            .sign_with_keys(&keys)
            .unwrap();
        let unmanaged = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tag(Tag::parse(["d", "upstream"]).unwrap())
            .custom_created_at(Timestamp::from(100))
            .sign_with_keys(&keys)
            .unwrap();
        let projection = fold_tracker_projects_at(
            [invalid, unmanaged],
            workspace,
            &keys.public_key().to_hex(),
            100,
        );
        assert!(projection.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            ProjectDiagnostic::InvalidManagedHead { coordinate, .. }
                if coordinate.ends_with(":invalid")
        )));
        assert!(projection.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            ProjectDiagnostic::UnmanagedCoordinate { coordinate }
                if coordinate.ends_with(":upstream")
        )));
    }

    #[test]
    fn project_fold_warns_for_a_tracker_head_not_authored_by_the_managed_signer() {
        let managed_signer = Keys::generate();
        let member = Keys::generate();
        let workspace = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let foreign = tracker_project_head(&member, workspace, project_id, "member-project", 100);
        let expected_coordinate = format!("30621:{}:member-project", member.public_key().to_hex());

        let projection = fold_tracker_projects_at(
            [foreign],
            workspace,
            &managed_signer.public_key().to_hex(),
            100,
        );

        assert!(projection.projects.is_empty());
        assert_eq!(
            projection.diagnostics,
            vec![ProjectDiagnostic::UnmanagedCoordinate {
                coordinate: expected_coordinate,
            }]
        );
    }

    #[test]
    fn project_fold_rejects_future_skewed_heads() {
        let keys = Keys::generate();
        let workspace = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let future = tracker_project_head(
            &keys,
            workspace,
            project_id,
            "future",
            100 + MAX_FUTURE_SKEW_SECONDS + 1,
        );
        let projection =
            fold_tracker_projects_at([future], workspace, &keys.public_key().to_hex(), 100);
        assert!(projection.projects.is_empty());
        assert!(matches!(
            projection.diagnostics.as_slice(),
            [ProjectDiagnostic::FutureSkew {
                created_at,
                max_accepted_at,
                ..
            }] if *created_at == 401 && *max_accepted_at == 400
        ));
    }
}
