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
| 40026 | `DISPATCH_RECORD` | Versioned planning, decisions, delivery, views and controls |
| 40027 | `DISPATCH_ADMISSION` | Immutable relay-signed per-event authorization evidence |
| 40028 | `DISPATCH_CLIENT` | Authenticated read/preparation command; never shared history |

All allocated kinds are workspace-channel scoped and MUST carry exactly one
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
`in_review`, `done`, and `cancelled`. All transitions are legal. A
status-changing revision SHOULD carry a reason; when present, the reason MUST
be non-empty. `done` and `cancelled` are terminal. Terminal transitions and
reopen transitions require the workspace-configured maintainer role.

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

The workspace signer is the pubkey in the `owner` row of the workspace
channel's verified kind-39002 membership snapshot. Tracker-manageable project
heads MUST be authored by that workspace signer. Heads authored by any other
pubkey remain referenceable by their NIP-MP coordinate with a warning, but
MUST NOT enter the manageable project projection.

Slug uniqueness is enforced under ASCII case folding by CLI/projection scope
among the workspace signer's managed projects, matching CLI slug resolution.
A rename publishes the same project UUID at the new `d` address and a NIP-09
deletion for the old kind-30621 address. A relay cannot enforce uniqueness
across signers. Cross-signer upstream NIP-MP heads may be referenced by
coordinate with a warning, but are not Tracker-manageable even when they carry
Tracker UUID/workspace tags.

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
request ID shared between kind 40023 and the task envelope.

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
correlation fields to absent. New authority-aware writers emit v3, described below. Release order is: shared SDK
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
event ID, so same-second events cannot be skipped. Durable CLI and Desktop consumers retain their verified history. The bounded
constrained-client command below is an explicit read path for mobile clients
that do not embed the Rust fold.


## Historical authority and admission receipts (40027)

Issue roots, revisions, comments and planning records retain the role held by
their signer when the relay accepted that exact signed event. On updated relays,
the original event and a kind-40027 receipt commit in one database transaction,
behind the same per-channel membership lock used by role changes. A retry of an
already stored source does not create another observation or upgrade its role.
The relay publishes the receipt before the source; both are already durable.

The strict receipt has `v:1`, `workspace`, `event_id`, `author`, `role`,
`observed_at` and `basis`. Its exact tags are `h` (workspace), `e` (source event)
and `p` (source author). The event timestamp equals the relay's `observed_at`,
independent of the source's author-controlled timestamp. Roles are `member`,
`maintainer`, or `none`; `none` is only valid for an explicit legacy baseline.
The only bases are `admission` and `legacy_baseline`. Clients verify the expected
workspace relay signer, signature, scope, source identity and clock. Contradictory
receipts fail closed for their target. An attested role never grants a current
role, read access, executor access or permission to create another event.

Unreceipted legacy events retain the compatibility policy of current membership,
with explicit coverage reporting. This is not proof of historical authorization.
Operators can preview `buzz-admin dispatch-baseline --workspace UUID` using the
configured community and durable relay key. The preview includes exact event IDs,
authors, currently observed roles and a digest. Supplying that digest through
`--apply-digest DIGEST` appends labeled baseline receipts atomically only if the
manifest still matches. No source events change. Removed authors receive `none`;
the tool never reconstructs an earlier role it did not observe. The command
refuses an ephemeral/wrong issuer and caps one reviewed baseline at 10,000 events.
A retry after an uncertain apply should first preview again; zero pending events
and the persisted receipts establish completion without another baseline.
Refresh client history after an operator baseline.

Receipts use relay observation time. Cold, warm and live readers hydrate each
new receipt's exact scoped source ID: otherwise a backdated source arriving after
a local timestamp cursor could be omitted. Missing sources are visible history
errors, not an empty-work claim. This contract seals issue/planning records and valid kind-9 task envelopes in their
execution channels. A baseline must include each execution channel whose legacy
history will authorize automation. Task-envelope authority additionally follows
the assignment chain below.

## Task-envelope v3 authority

V3 retains v2 correlation and adds `authority` with `assignment_event_id`,
`previous_event_id` and `reviewer`. An assignment root leaves the two event
references null. Every continuation names the original assignment and its accepted
predecessor. Task ID, assigner, executor, issue/request correlation and execution
channel are immutable for the attempt. Legacy readers remain explicit compatibility
readers; writers must not emit v3 to a harness that cannot parse it.

The assignment signer is its assigner. Execution updates belong to its executor
within the valid lifecycle. Verification requires a preceding `done` claim and a
nonempty evidence reference; the signer must be the assigner or designated reviewer
and must differ from the executor. The same independent authority can reject a
`done` result with `failed`, evidence and a nonempty explanation. Executors cannot
review their own completion into either result. `verified` and `failed` attempts cannot resume.
A later attempt is a new task ID. Signed rejected claims remain diagnostics, never
accepted current state. Display all accepted attempt history and keep claimed
completion separate from independent review and delivery observations.

## Planning records (40026)

Strict version-one records carry a stable `record_id`, zero-based `revision`,
`parent_event`, retained `request_id`, `archived` and a typed payload. Supported
payloads are shared views, decisions, project plans, imported-work activation,
delivery observations, capacity and automation controls. Revisions retain record
identity and subject and name the accepted predecessor. Unknown fields, duplicate
option IDs, invalid bounds and unauthorized changes are rejected by the shared SDK.

Creators or maintainers can edit ordinary records. Choosing or changing a resolved
decision requires its named decision maker. Activation, capacity and automation
controls require maintainer authority. Reviewed/accepted delivery observations
remain distinct maintainer-authored proof tiers. The latest singleton setting wins
before archival is applied, so archiving a setting cannot revive an older active one.

Automation is opt-in at the service host (`remote_controls`) and narrows the host
allowlist. Preview/paused/expired controls produce no work effects. Activation has
an expiry, exact selected issue/capture scope, per-cycle and total-action limits,
a current approver and expected service signer. Host reservations survive restart.
Fresh approval and audience checks precede each effect/replay. The accepted issue
link precedes an executor message. No unsealed or contradictory workspace history
can activate unattended work. Pausing or resuming preserves the original allowance.
Changing rule scope requires a new preview and a visibly separate allowance.

## Constrained-client command (40028)

This is a Nostr command accepted through either existing event transport after
signature, freshness, actor and `messages:read` scope checks. It requires exactly
one `h` workspace and current channel membership; token channel restrictions also
apply. Requests and responses are not stored or broadcast. The response uses the
existing command receipt `response:{...}` and is an authenticated relay projection,
not an independently signed delivery or verification receipt.

Content is `{"v":1,"request":{"operation":...}}`. Operations are `snapshot`,
`prepare_issue`, and `prepare_decision`. The shared SDK performs the same history,
role, conflict and decision folds used by Desktop/CLI. Snapshot pages use stable
issue UUIDs (100 per page); a detail read supplies one issue UUID. Terminal work is
included only when requested, and authorization coverage is explicit. A single
read-only repeatable database snapshot caps history at 20,000 records and 16 MiB
before materializing event contents. Oversized history or
response limits produce an error rather than a partial claim of completeness.

Preparation requires the exact issue/decision version shown to the user and a
retained request ID. It returns an unsigned standard 40021 or 40026 event. It does
not sign, publish, assign, wake or resolve work. The mobile client must validate
its scope and intent, sign locally, durably retain exact signed bytes, publish
through the ordinary path, then verify accepted ancestry before reporting success.
Membership and identity changes, conflicts and uncertain results must preserve the
user's draft and expose recovery. A relay without command support is explicitly
unavailable; it must not be rendered as an empty Dispatch workspace.

## Explicit conversation updates (kind 9)

A user may share an issue status, execution blocker/result, or decision in a
conversation. The standard message carries `t=dispatch-update`, an exact `h`
destination, a `d` identity and one `dispatch` tag with workspace, issue, referenced
change event and purpose. An optional `q` names the source message, with normal
thread tags. At most one explicit notification recipient is permitted. These
messages convey context; they never mutate an issue or establish review authority.

The stable key is SHA-256 over
`dispatch-notice-v1:workspace:issue:destination:source-or-empty:change-event:purpose`.
Purposes are `tracking`, `status`, `blocked`, `completion`, `decision_requested`,
and `decision_resolved`. The relay rejects mixed task markers and malformed keys,
checks current conversation membership, and serializes duplicate-key insertion
under the channel membership lock. Identical event retries are idempotent. A
second signed message for the same key is rejected even after redaction.

Clients preview message text and check the destination audience can read all
included source channels. They retain the exact signed event before publication,
then read back and validate its signed descriptor before clearing recovery. A
concurrent already-shared update is displayed using its actual accepted message.
A read-only snapshot advertises `capabilities.dispatchNotice=1`; clients require
this capability before publishing. `taskAuthority=3` describes relay/SDK support,
not proof that a remote executor has upgraded its serving harness.

## Recovery and client presentation

Durable pending writes are scoped to relay, signing identity and workspace. Lost
acknowledgement retries reuse the exact original signed event and verify accepted
ancestry. A stale predecessor cannot be silently rebased. A user may archive a
pending write as a local draft only after its recovery copy is durably saved;
archival does not recall a message already published. The UI offers current-history
review, readable copying, recovery export and local removal. Storage read errors
must fail closed, never be interpreted as an empty outbox.

Assignment recovery checks the accepted issue link before notifying an executor,
rechecks every source audience, and holds when capacity/dependency conditions have
changed. An unresolved existing task link blocks another execution attempt until
it is reconciled. Retained review and conversation updates similarly freeze the
reviewed source event. Issue completion, executor completion, independent review,
merge, installation and observed acceptance remain separate states.

## Built-in relay automation authorizations (kind 40026)

These payloads use the existing immutable `DispatchRecord` wrapper: `v: 1`, a
stable `record_id`, `revision` starting at zero, `parent_event` null on the root
and equal to the accepted predecessor on each revision, a fresh `request_id`,
`archived`, and `payload`. Update the same record to change or pause a setting;
archiving the latest setting does not revive an older grant. Normal admission
receipts seal the author's role at write time. The relay additionally requires
the current record author to remain an **owner or admin**, not merely a bot or
an admitted former maintainer, at effect time.

### Capture

The signed event's `h` tag is the source `channel`. That channel's owner/admin
signs the record. The signer must also belong to `workspace`; its audience may
not exceed the source audience. `project` is optional (null or an existing
project UUID). The reaction's stored channel and message target are authoritative;
reaction-controlled text cannot select the workspace.

```json
{
  "type": "capture",
  "channel": "c2e8c9dd-f748-47c2-b56e-749ae34d1b62",
  "workspace": "8dd44714-7598-4710-92a8-77d4effbf39a",
  "project": null,
  "enabled": true
}
```

### Unowned alerts

The `h` tag is `workspace`; a workspace owner/admin signs. `channel: null` uses
each issue's home channel (or its workspace when no home is set). An override
must preserve the workspace audience and include the author. `after_hours` is
1–8760 and must match the workflow parameter. Nonterminal unassigned issues
qualify by time in their current state. Each `(issue, threshold)` fires once,
even across restart or repeated schedule ticks. Changing ownership never occurs.
Age starts at the relay-signed admission receipt for the canonical root or most
recent actual status change. Author timestamps and unrelated edits cannot age
or reset the timer. Only relay-signed prior action records can suppress a
duplicate alert; a member-authored kind-40023 claim is not execution evidence.

```json
{
  "type": "unowned_alert",
  "workspace": "8dd44714-7598-4710-92a8-77d4effbf39a",
  "channel": null,
  "after_hours": 24,
  "enabled": true
}
```

### Autopilot

The `h` tag is `workspace`; a workspace owner/admin signs. `agents` contains
1–32 distinct public keys. `at_a_time` is per executor (default 2, range 1–32)
and `per_day` is per project per UTC day (default 10, range 1–1000). An existing
capacity record may further lower an executor's limit or pause them.
`reviewer_required` must be true. The issue owner (accountable assignee, falling
back to a non-relay root author) reviews and cannot equal the selected executor.
The executor and reviewer must belong to both the workspace and destination;
the issue body is never copied to a broader destination audience.

```json
{
  "type": "autopilot",
  "workspace": "8dd44714-7598-4710-92a8-77d4effbf39a",
  "project": "31334708-f5ba-4d7e-b7d6-8bd63984de30",
  "agents": ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
  "label": null,
  "at_a_time": 2,
  "per_day": 10,
  "reviewer_required": true,
  "enabled": true,
  "activation_id": "624f136a-e4e9-4af6-942e-d722ca9bc1e0",
  "confirmation_required": true,
  "confirmation_event": null
}
```

Use a real agent public key in `agents`. A fresh activation begins with
`confirmation_required: true`. The relay admission time of its first accepted revision fixes the start of
the first hour, so backdating the signed policy cannot bypass warm-up. The third assignment within that hour emits a durable confirmation
outcome; further assignments stop until an admin revision sets the flag false
and `confirmation_event` to that exact third kind-40023 assignment event ID.
An earlier confirmation, a timestamp coincidence, or an unrelated event cannot
satisfy this check. Keep `activation_id` unchanged when confirming. If fewer than
three assignments occur during the first hour, the warm-up limit expires. If
three occurred, confirmation remains pending even after the hour ends. Pausing
only prevents future assignments.

Only Todo issues in the named project, matching `label` when present and with
no nonterminal blocking issue, qualify. Imported issues require an active signed
activation record. Unfinished attempts, including reported-complete work awaiting
independent review, consume capacity. The relay preserves the accountable owner
and issues an unchanged v3 task envelope to the home channel. Issue links, envelope,
admission receipts, action record and `(issue, observed revision)` claim commit
atomically before notification. Per-day counts and first-hour claims survive
restart and policy revisions.

### Action history and durable claims (kind 40023)

Existing issue-bound `DispatchAction` records retain their `assign`/`escalate`
schema and request UUIDs, preserving the optional dispatcher's reader contract.
Capture results, missing/revoked-authority skips and first-hour confirmation use
a separately tagged `t: dispatch-workflow-outcome` record:

```json
{
  "v": 1,
  "type": "workflow_outcome",
  "action": "dispatch.create_issue",
  "request_key": "skip:create_issue:<source-event-id>:no authorization record",
  "source": "<source-event-id>",
  "result": "skipped: no authorization record",
  "confirmation_required": false,
  "confirmation_event": null
}
```

These records are signed by the relay. The `h` tag is the workflow's scope for
skips, the receiving workspace for successful capture, and the project workspace
for confirmation. Trust the relay signature before displaying these as automation
results. A skipped source does not consume its successful-capture claim: an admin
can authorize a later retry. Source capture claims are scoped to the original
conversation, so changing its destination does not recapture the message.
Cross-writer capture deduplication honors an accepted canonical issue root only
while its author is a current workspace member. It emits a source-idempotent
skipped outcome naming that existing issue, making a false member claim visible
instead of silently suppressing capture. The durable relay capture claim remains
the authority for captures already performed by the relay itself.

Migration **0055** adds `dispatch_workflow_effects`, keyed by community, scope
and effect key, protected by the community write fence and included in community
deletion. Scope membership locks serialize policy changes, normal Tracker writes
and concurrent automation. The transaction stores claims and their signed effects
together. A snapshot is bounded to 64 channels, 20,000 events/16 MiB, and 100,000
ledger entries; exceeding a bound fails visibly instead of acting on partial
history. A batch handles at most 100 alert effects or 60 assignment/skip effects;
subsequent ticks continue past already claimed effects.
Scope discovery precedes the locked snapshot. A concurrent home-channel move can
therefore abort a tick with `Dispatch event scope mismatch`; this fails closed
and the next tick discovers the new scope.

### Task lifecycle projects onto issue state

An accepted v3 `picked_up` event automatically adds a Tracker revision to
In Progress; accepted `done` adds In Review. Current signed assignment authority,
reciprocal task-event links and the independent-review contract remain intact.
Forged, stale and superseded attempts cannot move the board. The transition keeps
the issue owner and records the relay pickup time through the ordinary status
activity. Once the source event is durably stored, projection failures are logged
at error level and never change the successful ACK (including duplicate ACKs).
Admitted task history is the durable retry source: the next scheduled Dispatch
action reconciles up to 32 pending current pickup/done states in its transaction,
even if new assignments are paused. Existing snapshot bounds still apply;
oversized or unavailable history remains pending and fails visibly until repaired.
Ledger claims prevent duplicate revisions and prevent a consumed lifecycle event
from undoing subsequent manual board changes. The relay uses the existing
Tracker monotonic revision timestamp rule so same-second acceptance cannot lose
to a prior revision's lexical event-ID ordering.
