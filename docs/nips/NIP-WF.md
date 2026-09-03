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
