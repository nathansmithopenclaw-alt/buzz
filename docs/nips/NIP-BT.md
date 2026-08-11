# NIP-BT: Workspace Tracker Events

`draft` `optional`

## Abstract

This document defines an append-only, workspace-scoped issue protocol for
Nostr collaboration systems. It allocates immutable issue, revision, and
comment kinds, a project-activity kind, plus dispatcher-action and
service-heartbeat kinds. Current
state is a deterministic projection produced by a strict shared folder; relay
arrival order and replaceable-event semantics do not select state.

## Kind allocation

| Kind | Name | Retention |
|---:|---|---|
| 40020 | `TRACKER_ISSUE` | Immutable issue root |
| 40021 | `TRACKER_REVISION` | Immutable sparse revision |
| 40022 | `TRACKER_COMMENT` | Immutable comment; NIP-09 may redact it |
| 40023 | `TRACKER_DISPATCH_ACTION` | Immutable idempotent automation action |
| 40024 | `TRACKER_SERVICE_HEARTBEAT` | Latest heartbeat per service identity |
| 40025 | `TRACKER_PROJECT_ACTIVITY` | Immutable project update/rename history |

All six kinds are workspace-channel scoped and MUST carry exactly one
`["h", "<tracker-stream-uuid>"]` tag. A deployment provides one Tracker
stream UUID through workspace configuration; protocol implementations MUST
NOT hardcode organization names, stream IDs, identifier prefixes, roles, or
routing tables.

Kinds 40020, 40021, and 40022 additionally carry exactly one issue identity
tag `["d", "<uuid-v4-or-v5>"]` and exactly one discovery tag
`["t", "buzz-tracker"]`. The issue root event ID remains part of every state
reference so an accidental duplicate UUID cannot retarget a relation.

## Issue root (40020)

The root is strict JSON. Unknown fields, duplicate singleton tags, malformed
UUIDs, content/tag disagreement, non-normalized labels, and unknown schema
versions are invalid. Version 1 contains:

```json
{
  "v": 1,
  "op": "create",
  "issue_id": "<uuid-v4>",
  "identifier": "BT-123",
  "title": "Short title",
  "body": "Markdown",
  "status": "triage",
  "priority": null,
  "assignee": null,
  "project_id": null,
  "due": null,
  "estimate_minutes": null,
  "home_channel_id": null,
  "source_event_id": null,
  "workflow": "default-v1",
  "labels": ["area:relay"],
  "external_refs": [],
  "request_id": null,
  "import": null
}
```

The seven stable status IDs are `triage`, `backlog`, `todo`, `in_progress`,
`in_review`, `done`, and `cancelled`. All transitions are legal, but every
status-changing revision carries a non-empty reason. `done` and `cancelled`
are terminal. Terminal transitions and reopen transitions require the
workspace-configured maintainer role.

`labels` contains at most 16 unique values. Each is already trimmed,
lowercase, and at most 64 UTF-8 bytes, and each has a matching repeatable
`["l", "<label>"]` tag. `estimate_minutes`, when present, is positive. A due
value is `{ "value": <unix-seconds>, "precision": "instant|date",
"timezone": "<IANA-zone>" }`. Implementations preserve the precision and
zone rather than converting a date-only commitment into an end-of-day
instant. Estimate projections use `["estimate", "<positive-integer>",
"minutes"]`; date-precision due epochs resolve to local midnight in the
declared zone.

Each root external reference has a matching
`["external_ref", "<type>", "<ref>"]` tag. Revision operations project as
`["external_ref", "add|remove", "<type>", "<ref>"]`; relation operations
project as `["relation", "add|remove", "blocks|duplicate_of|parent",
"<target-root-event-id>"]`. Nullable-field removals project as repeatable
`["clear", "<field>"]` tags.

`request_id`, when present, is indexed by a matching
`["request", "<uuid>"]` tag. It is an idempotency correlation key, not issue
identity. `identifier` is revisable, but renumbering requires maintainer role.
When multiple issues in one stream claim an identifier, the issue whose root
has the smaller `(created_at, event_id)` is canonical; all other claims remain
visible as collision diagnostics.

Importer-generated issue and request IDs MAY use UUIDv5. Interactive CLI
writes use UUIDv4. Import provenance is strict typed data: source system and
external ID, source URL/timestamps, import timestamp, archived state, source
estimate points, export SHA-256, importer version, original author, and legacy
pilot labels. It never changes the Nostr signer or event `created_at`.

## Revisions (40021)

A revision carries exactly one root marker, parent marker, and revision tag:

```text
["e", "<root-event-id>", "", "root"]
["e", "<parent-root-or-revision-id>", "", "reply"]
["revision", "<positive-integer>"]
```

Its strict JSON is a sparse change:

```json
{
  "v": 1,
  "op": "revise",
  "issue_id": "<uuid-v4>",
  "root_event": "<event-id>",
  "parent_event": "<event-id>",
  "revision": 3,
  "set": {"status": "in_progress"},
  "clear": [],
  "relations": {"add": [], "remove": []},
  "external_refs": {"add": [], "remove": []},
  "reason": "Execution started",
  "request_id": "<uuid>",
  "import": null
}
```

The revision integer MUST equal the parent's integer plus one, with the root
defined as revision zero. It is stale-write and conflict evidence; it does not
outvote event recency. Empty revisions and fields present in both `set` and
`clear` are invalid.

External references are identified by `(type, ref)` and change only through
add/remove operations. Consumers deterministically apply valid operations, so
two concurrent adds survive rather than replacing a whole list.

The only canonical stored relation types are `blocks`, `duplicate_of`, and
`parent`. `blocked_by` and `child` are inverse query/build vocabulary and MUST
be canonicalized before signing. Self-edges are invalid. Removing an absent
edge is an idempotent no-op. Targets are Tracker roots in the same stream.

## Comments (40022)

Comments carry a root marker and may carry a reply marker. Their strict JSON
is `{ "v": 1, "issue_id": "<uuid>", "body": "Markdown",
"request_id": null, "import": null }`. Notification `p` tags do not grant
authority. Comments never select or advance the state tip.
The shared folder retains valid comments in `(created_at, event_id)` order,
separate from issue state.

A valid NIP-09 deletion may redact a comment. Projections retain a redaction
marker at the comment's original activity position. NIP-09 deletion requests
targeting 40020 roots or 40021 revisions MUST be rejected; administrative
erasure is a distinct governed operation and does not rewrite normal history.

## Projects and project activity (30621, 40025)

Tracker reuses the NIP-MP kind-30621 project entity. Its `d` tag is the
human-readable slug. A Tracker-managed head additionally carries
`["buzz-project-id", "<immutable-uuid>"]` and
`["buzz-workspace", "<tracker-stream-uuid>"]`, plus ordered workflow status
tags and optional channel/verification defaults. The shared SDK projection
maps the immutable UUID to the signer's current coordinate at read time;
issues store only that UUID.

Slug uniqueness is enforced under ASCII case folding by CLI/projection scope
among the workspace signer's managed projects, matching CLI slug resolution.
A rename publishes the same project UUID at the new `d` address and a NIP-09
deletion for the old kind-30621 address. A relay cannot enforce uniqueness
across signers. Cross-signer upstream NIP-MP heads without Tracker
UUID/workspace tags may be referenced by coordinate with a warning, but are
not Tracker-manageable.

A rename interrupted after the new head is accepted but before the old-address
deletion is accepted leaves duplicate heads for one immutable project UUID.
Recovery first retries the rename with the same request ID. If that request ID
is irretrievably unavailable, an operator must identify the newest valid head
from the `DuplicateProjectId` diagnostic, publish a same-signer NIP-09 deletion
whose `a` tag targets only the stale 30621 address, and verify that the
diagnostic clears. The newest address must never be deleted merely to silence
the diagnostic; immutable kind-40025 activity remains history, not deletion
authority.

Every project create, update, or rename also publishes immutable kind 40025.
Its strict content contains `v`, `op`, `project_id`, `request_id`, and complete
before/after snapshots. Tags include `h`, `t:buzz-tracker-project-activity`,
`d:<project-uuid>`, and `request`. This activity stream—not retention of old
replaceable heads—is the readable project revision history.

## Dispatcher actions (40023)

Dispatcher actions are signed by a dedicated workspace-configured service
identity. They carry `h`, `t:buzz-tracker-dispatch`, issue `d`, and
`["request", "<uuid>"]`. Strict content records the issue/root IDs, action
type, threshold identity, observed canonical tip, and result. The tuple
`(dispatcher pubkey, request_id)` is the idempotency key. Escalation threshold
IDs are stable configuration values, allowing at-most-once firing per issue,
threshold, and qualifying state interval without organization-specific wire
constants.

Self-authored mentions are not a wake-up mechanism: harnesses may discard
them. The dispatcher uses its own signing identity and timer-driven
subscriptions, then publishes human-facing notifications with the intended
recipient's `p` tag. It may assign a task-envelope itself using one dispatch
request ID shared between kind 40023 and task-envelope v2.

## Service heartbeat (40024)

The dispatcher and graph-edge watchdog share one timer/subscription core and
publish a service heartbeat containing `v`, `service_id`, `instance_id`,
`observed_at`, `cursor`, and health details. Tags are `h`,
`t:buzz-tracker-heartbeat`, and `["d", "<service-id>"]`. Consumers select the
maximum `(created_at, event_id)` per `(stream, service signer, service_id)` and
surface an expired heartbeat on a workspace canvas. The watchdog is external
to the service it watches; a dead process cannot author its own death notice.

## Deterministic fold

For each `(stream UUID, issue UUID)`:

1. Verify signature, kind, strict schema, tags, field bounds, clock skew, and
   workspace role. Events more than 300 seconds in the future are invalid.
2. Select the root with minimum `(created_at, event_id)` and surface every
   duplicate-root collision.
3. Validate parent reachability, exact parent-relative revision integer, and
   semantic/role constraints. A child of an invalid revision is invalid.
4. Select the canonical revision tip with maximum `(created_at, event_id)`
   among valid revisions and replay its parent chain. Chain depth NEVER beats
   recency. Other valid branches remain visible conflict evidence.
5. Apply valid external-reference set operations deterministically so
   concurrent adds survive. Fold comments separately by
   `(created_at, event_id)`.

Relay arrival order, local wall-clock receipt time, and parameterized
replaceable semantics MUST NOT affect this result.

## Authorization

The fold accepts roles from workspace configuration. Members may create,
edit, relate, assign, and comment. Maintainers additionally may renumber,
transition into a terminal state, and reopen a terminal issue. The signing
pubkey is the actor; creator and assignee fields do not confer authority.
Relays enforce stream membership at ingress, and every bridge/projector
re-checks authorization through this shared fold.

## Task-envelope v2 coordination

Task-envelope v2 adds nullable `tracker_issue_id` and
`dispatch_request_id` fields. Its JSON remains `deny_unknown_fields` and
changes `v` from 1 to 2. Therefore an old reader fails loudly on both the
version and new fields instead of silently dropping tracker correlation.
New readers continue to parse historical v1 envelopes and normalize their two
correlation fields to absent; they emit only v2. Release order is: shared SDK
readers and relay kind support, then CLI/harness
writers, then dispatcher activation. Mixed-version writers MUST NOT publish v2
until the consuming harness has the v2 SDK.

## Subscription shapes

Consumers bootstrap one stream with filters for kinds 40020–40025 plus `#h`,
then incrementally subscribe by the same stream. Detail reads add `#d`; request
reconciliation adds `#request`; label/triage queries add `#l` but always feed
the complete issue history to the shared folder before claiming current
state. Raw relay filters are candidate indexes, not current-state queries.
Consumers persist a local projection and a composite `(created_at,event_id)`
cursor. Refresh queries are inclusive at the last timestamp and deduplicate by
event ID, so same-second events cannot be skipped. Naively refetching and
refolding the full stream for every command is not conformant.
