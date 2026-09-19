# NIP-WF: Workflow Approval Gates

`kind:46010` is a relay-authored, persistent request for an approval decision.
It is channel-scoped with `h` when the workflow belongs to a channel and is
always addressed to exactly one approver with `p`.

## Approval request (`kind:46010`)

Required tags:

- `d`: an opaque, lowercase 64-hex approval reference. Implementations derive
  it as SHA-256 of a high-entropy, transient approval capability. The raw
  capability MUST NOT appear in any event tag, content, log, or persisted row.
- `p`: the designated approver pubkey. A workflow whose `from` policy is `any`
  resolves and persists the workflow owner as the concrete approver; it does
  not create a community-wide action policy.
- `workflow`: workflow UUID.
- `run`: workflow-run UUID.
- `step`: workflow step identifier.
- `revision`: lowercase 64-hex SHA-256 hash of the canonical workflow
  definition. The persisted approval row binds this revision and the exact
  `(step index, step identifier)` that may resume.
- `expiration`: Unix timestamp after which the request cannot be resolved.

Relays MUST treat `kind:46010` as both p-gated and result-gated. A request may
be returned only to an authenticated reader whose pubkey equals its `p` tag.
This applies to kind filters, kindless or ID-based reads, counts, full-text
search, and live fan-out.

## Decision (`kind:46030` or `kind:46031`)

The approver grants with `kind:46030` or denies with `kind:46031`. The decision
places the request's opaque approval reference directly in its `d` tag. Clients
MUST accept an approval reference, not a raw token, and MUST NOT hash the
reference again. Optional decision notes are event content.

The relay resolves the referenced row inside the connection's community and
authorizes the event signer against the row's stored approver before changing
state. Before granting, it MUST verify that the current workflow revision and
step still match the persisted binding, then repeat that check immediately
before resuming execution. Possession of an approval reference is not authority.

## Expiry

Relays MUST sweep unanswered requests after `expiration`. The transition is
idempotent: a pending approval becomes `expired`, and its `waiting_approval`
run becomes terminal `failed` with error code `approval_expired`. A repeated or
concurrent sweep MUST NOT transition either row again.

## Dispatch actions

The relay implements these actions through `ActionSink`, using direct database
transactions. No sidecar or HTTP loopback is required. Definitions still require
the normal workflow owner authority, and every Dispatch effect additionally
requires the current signed NIP-BT authorization described below.

| Action | Trigger | Parameters |
| --- | --- | --- |
| `dispatch.create_issue` | `reaction_added`, emoji `📌` | None |
| `dispatch.escalate` | `schedule` | `after_hours`: integer 1–8760, default 24 |
| `dispatch.assign` | `schedule` | None; project scope and limits come from signed Autopilot records |

The desktop capture definition is accepted unchanged:

```json
{
  "name": "dispatch-capture",
  "trigger": {
    "on": "reaction_added",
    "emoji": "📌",
    "filter": "trigger_channel_id == \"c2e8c9dd-f748-47c2-b56e-749ae34d1b62\""
  },
  "steps": [{"id": "capture", "action": "dispatch.create_issue"}]
}
```

A separate signed `capture` record names the receiving Tracker workspace.
Enabling the workflow alone does not authorize capture. Un-pinning does not
remove the issue. The source message supplies the title's first line, bounded
to 256 UTF-8 bytes, and a source link; captured work enters Triage.

Scheduled actions reuse the existing scheduled-fire claim table. For example:

```yaml
name: dispatch-ready-work
trigger:
  on: schedule
  interval: 1m
steps:
  - id: assign
    action: dispatch.assign
```

An alert definition uses `action: dispatch.escalate` with `after_hours: 24`.
The threshold must equal the signed `unowned_alert` authorization. A tick
coalesces eligible issues into one digest per destination channel. Assignment
and alerting remain separate actions; an alert never assigns.

Unknown actions, invalid thresholds and incompatible trigger/action combinations
are rejected during definition validation. At execution, the relay rechecks the
current definition and enabled state under a database lock, current policy-signer
roles, source/destination audiences, and accepted issue/task history. Missing or
revoked grants produce an idempotent kind-40023 skipped outcome. Storage errors
fail the run rather than returning an empty successful projection.
