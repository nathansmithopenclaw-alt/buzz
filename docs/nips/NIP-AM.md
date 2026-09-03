NIP-AM
======

Agent Turn Metrics
------------------

`draft` `optional` `relay`

This NIP defines a durable, encrypted event kind for recording per-turn token
usage and estimated cost of AI agent sessions. An agent publishes one
`kind:44200` event per completed turn, NIP-44 encrypted to its owner, so the
owner can account for token usage across agents and harnesses without the
relay — or any third party — learning what the agent did or what it cost.

## Motivation

AI agent harnesses consume model tokens on every turn. Owners running fleets
of agents need durable, harness-independent usage accounting — the equivalent
of a metered bill — for cost attribution, budgeting, and capacity planning.

[NIP-AO](NIP-AO.md) (kind 24200) already streams encrypted session telemetry
between agent and owner, but it is deliberately ephemeral: relays MUST NOT
persist it, so it cannot answer "how many tokens did my agents use last
week?". Transcript-grade durable telemetry is explicitly out of scope — the
persistence-averse reasoning behind NIP-AO's ephemerality contract applies to
conversation content, not to a small usage record. Kind 44200 stores only the
metric: token counts, an estimated cost, and correlation identifiers, all
encrypted to the owner.

## Definitions

- **Agent**: an AI process with its own Nostr keypair, executing sessions on
  behalf of an owner.
- **Owner**: the human (or system) whose pubkey the agent was provisioned under.
- **Turn**: one prompt→response cycle of an agent session, as bounded by the
  harness (e.g. one ACP `session/prompt` round trip).
- **Turn metric**: a single kind 44200 event recording the usage of one turn.

## Event

`kind:44200` is a regular event by Buzz convention (alongside 44100/44101):
stored,
append-only, never replaced. Each completed turn produces exactly one event.

```json
{
  "kind": 44200,
  "pubkey": "<agent_pubkey>",
  "created_at": <unix_timestamp>,
  "content": "<NIP-44 v2 ciphertext>",
  "tags": [
    ["p",     "<owner_pubkey>"],
    ["agent", "<agent_pubkey>"]
  ],
  "sig": "..."
}
```

Events MUST have exactly one `p` tag (the owner) and exactly one `agent` tag
(equal to `pubkey`). The tag layout deliberately mirrors NIP-AO telemetry
frames so existing owner-scoped tooling applies unchanged.

No channel (`h`) tag is used. The channel a turn served is private usage
metadata and lives inside the encrypted payload; keeping it out of the tags
avoids leaking per-channel activity rates to the relay operator and keeps the
event community-global (owner-scoped) rather than channel-scoped.

## Encryption

`content` MUST be encrypted with NIP-44 v2 using `(agent_privkey,
owner_pubkey)` — identical to NIP-AO telemetry. Plaintext SHOULD be zeroized
after encrypt/decrypt. Decrypted payload MUST NOT exceed 65,535 bytes
(payloads are typically well under 1 KB).

## Decrypted Payload

The `content` field decrypts to a UTF-8 JSON object:

```jsonc
{
  "harness":   "goose",                  // REQUIRED: harness identifier
  "model":     "claude-sonnet-4-5",      // model id, or null if unknown
  "channelId": "<channel_uuid>" | null,
  "sessionId": "<session_id>"  | null,   // REQUIRED when "cumulative" is present
  "turnId":    "<turn_id>"     | null,
  "turnSeq":   17 | null,                // REQUIRED when "cumulative" is present
  "timestamp": "2026-07-01T20:11:03.213Z", // REQUIRED: RFC 3339, end of turn

  // Usage for THIS turn (computed delta). Fields are null when the harness
  // does not report them — a null MUST NOT be recorded or summed as zero.
  // Exception: cache fields (cacheReadTokens, cacheWriteTokens) MUST be
  // omitted rather than null when unavailable — see "Numeric validity" below.
  "turn": {
    "inputTokens":  1234  | null,
    "outputTokens": 567   | null,
    "totalTokens":  1801  | null,
    "costUsd":      0.0123 | null        // estimated
  },

  // Session-cumulative usage as reported at the end of this turn.
  "cumulative": {
    "inputTokens":  45210 | null,
    "outputTokens": 9876  | null,
    "totalTokens":  55086 | null,
    "costUsd":      0.41  | null         // estimated
  },

  // false when the publisher could not observe the previous turn's
  // cumulative baseline (e.g. harness restart mid-session), making the
  // "turn" object unreliable for this event.
  "deltaReliable": true,

  // Billing identity, present only when the publisher can prove applicability
  // from the actual endpoint (official provider API) and the actually-requested
  // model for the usage represented. Omit this field when applicability cannot
  // be proven — it is never inferred from the configured/session "model" field.
  // Consumers MUST treat omission as "price unknown"; they MUST NOT infer a
  // price from the session "model" field. pricingIdentity is OPTIONAL but NOT
  // nullable: when present, "authority" and "model" MUST be non-null strings;
  // "cacheClass" is omitted (not null) when it is not applicable.
  "pricingIdentity": {          // OPTIONAL; omit entirely if unproven
    "authority":  "api.anthropic.com",  // billing authority (not transport provider)
    "model":      "claude-sonnet-4-5",           // actually-requested billable model id
    "cacheClass": "ephemeral"                    // cache-write class; omit when not applicable
  },

  "stopReason": "end_turn",              // optional

  // Context state at the end of this turn, as reported by the harness. These
  // are STATE readings, not usage deltas: they are not summable across turns
  // and MUST NOT be added to any token total. Omitted when unreported.
  "contextTokens": 145213,     // OPTIONAL; context occupied after this turn
  "contextWindow": 200000,     // OPTIONAL; the model's context window

  // Harness-side provenance: how the publisher came to run this turn. All
  // three are OPTIONAL and describe the publisher, not the model.
  "attempt": 1,                // OPTIONAL; 1-based delivery attempt
  "isReplay": false,           // OPTIONAL; every trigger predates publisher start
  "sessionCreationCause": "rotation_turn_limit"  // OPTIONAL
}
```

`harness` and `timestamp` are REQUIRED. All other fields are OPTIONAL or
nullable, except as constrained below: `pricingIdentity` is optional but not
nullable (omit it entirely rather than set it to null), and so are
`contextTokens`, `contextWindow`, `attempt`, `isReplay`, and
`sessionCreationCause` — each is omitted rather than set to null, `0`, or
`false` when the publisher did not observe it. Consumers MUST ignore unknown
fields (forward compatibility).

### Ordering and delta recomputation

When a `cumulative` object is present, `sessionId` and `turnSeq` are
REQUIRED. `turnSeq` is a per-session monotonically increasing integer
starting at any value, incremented by the publisher on every published turn
metric for that session; a publisher restart that loses the counter MUST
start a new `sessionId` rather than reuse the old one with a reset `turnSeq`.
Cumulative values form a series only *within* one `sessionId`, ordered by
`turnSeq` — consumers MUST NOT diff cumulative values across different
`sessionId`s, and MUST NOT rely on `created_at` (seconds precision, ambiguous
for same-second turns) for ordering within a session.

If a consumer recomputing deltas observes a cumulative counter that decreases
between consecutive `turnSeq` values (counter reset, harness bug), it MUST
treat the affected turn's usage as unknown (null), not as negative usage.
Publishers likewise MUST NOT emit negative values in `turn`; when the
computed delta would be negative or the previous baseline is unknown, the
publisher sets the affected `turn` counters to null and `deltaReliable:
false`.

Where the harness reports only cumulative counters, the publisher computes
`turn` as the difference between consecutive cumulative snapshots within one
session. Consumers doing exact accounting SHOULD prefer recomputing deltas
from consecutive `cumulative` values and treat `turn` as a convenience.

### Numeric validity and token semantics

All token counts MUST be non-negative integers. `costUsd` MUST be a finite,
non-negative number. `totalTokens` is the harness- or provider-reported
total when available; publishers MUST NOT derive it by summing `inputTokens`
and `outputTokens` (providers may count categories a simple sum misses) —
when no total is reported, `totalTokens` is null. `inputTokens` is the
inclusive input-side total: where the provider reports cache reads/writes
separately (e.g. Anthropic `cache_read_input_tokens` /
`cache_creation_input_tokens`), the publisher folds them into `inputTokens`.
Where the provider exposes a cache component, publishers SHOULD report it in
the optional `cacheReadTokens` / `cacheWriteTokens` fields inside `turn` and
`cumulative`; these are informational subsets of `inputTokens`, not additions
to it. Publishers MUST preserve an explicit zero when the provider reports
zero and MUST omit the field (never null or fabricated zero) when that
component is unavailable to the publisher — including when the provider
supports the component but the harness does not surface it. Treating an
unreported category as zero is incorrect. Note: the payload-wide null
guidance above does not apply to these cache fields; omission is the only
valid representation for an unavailable cache component.

`costUsd` values are estimates (provider list prices at publish time, or a
harness-reported estimate). They are advisory, not billing records.

`pricingIdentity`, when present, identifies the billing authority and
actually-requested model for the usage represented by this event. `authority`
is a registered billing-namespace identifier: exact lowercase hostname, no
scheme, no path, no trailing slash (registered values: `api.anthropic.com`,
`api.openai.com`, `openrouter.ai`; the set extends only by amendment to this
NIP). Pricing lookup is an exact string match on `(authority, model)` — any
deviation loses the price. It is a billing namespace, distinct from the
runtime transport provider. `model` is the
billable model identifier as resolved at the point the request was made, not
the configured/session model. `cacheClass`
is the cache-write class when applicable (e.g. `ephemeral`). Publishers MUST
omit `pricingIdentity` when any of the following apply: the request was routed
through a custom or overridden base URL, a gateway (unless the gateway is the
named billing authority), or an unresolved alias; the billed model identity
cannot be confirmed — for direct connections to an official allowlisted
endpoint, proof is the actually-requested resolved model; for all other routes,
the response MUST supply authoritative billing identity; or the usage
represented within this turn contains contributions from more than one billing
identity (including a mix of identity-bearing and unresolved contributions).
The existing `model` field
retains its non-billing semantics (configured/session model) and is never
overloaded by `pricingIdentity`. Consumers MUST treat omission of
`pricingIdentity` as "price unknown". Consumers MAY recompute cost estimates
using the billing identity and a pricing manifest; they MUST retain
the provenance of any cost value (e.g. `manifest-estimated`, `wire-reported`).
Consumers MUST NOT merge manifest-estimated and wire-reported costs into an
unlabeled total.

`stopReason`, when present, MUST be one of `end_turn`, `max_tokens`,
`cancelled`, `error`, `unknown`. Consumers MUST treat unrecognized
`stopReason` values as `unknown`; the token counts remain valid.

### Context state

`contextTokens` and `contextWindow` describe the conversation's context
occupancy and the model's context-window size at the end of the turn. They are
OPTIONAL and MUST be omitted when the harness does not observe them.

Both are **state readings, not usage deltas.** A consumer MUST NOT sum them
across turns, add them to any token counter, or treat them as a component of
`inputTokens`. The meaningful aggregate over a set of turns is a maximum (peak
occupancy), not a total.

Publishers MUST NOT publish `0` for either field to mean "not tracked". A
harness that reports a zero context window on the wire because it tracks none
MUST have that normalized to omission before publication: a completed turn
always occupies context, and a zero-width window is not a measurement. This is
a deliberate exception to the "preserve an explicit zero" rule that governs the
cache fields — there, zero is a provider-confirmed count; here, zero is only
ever an absence in disguise.

### Turn provenance

Three OPTIONAL fields record how the publisher came to run this turn. They
describe the *harness*, not the model or the provider, so a publisher that
implements them SHOULD populate them identically across every adapter it drives.

`attempt` is the 1-based delivery attempt of the work that triggered this turn:
`1` on first delivery, `2` on the first retry, and so on. Omit it when the
publisher has no attempt notion for the turn — for example a self-scheduled
heartbeat that is never retried. Consumers MUST NOT treat `attempt > 1` as
proof of duplicated token spend: a retry may follow a failure that never
reached the model at all, so the field bounds an upper estimate of waste and
does not measure it. A consumer displaying a retry count MUST also expose how
many turns carried the field, because zero retries among zero reporting turns
is not evidence that no retries occurred.

`isReplay` is `true` when **every** event that triggered this turn predates the
publisher's own startup — i.e. the turn happened only because the publisher
restarted and replayed a bounded historical window. A batch mixing replayed and
live triggers is NOT a replay: that turn would have happened anyway, so its
tokens are not attributable to the restart. Publishers MUST omit the field
rather than publish `false` when they have no replay notion for the turn.

The field is scoped to a publisher's own startup replay. It does NOT claim to
identify a duplicate delivered by a transport-level reconnect backfill after
the publisher started; a consumer counting restart waste must not read it as a
general duplicate-delivery flag.

`sessionCreationCause` explains why the session this turn ran in had to be
created. Publishers SHOULD emit it on **every** turn of a session, not only the
first, so that a session's cache-write cost attributes to its cause and later
turns show how far that session amortized. Its value is one of:

| Value | Meaning |
|---|---|
| `worker_spawn` | The publisher process held no session state at all — a fresh or restarted process. |
| `scope_first_turn` | The process already had sessions for other scopes; this scope had none yet. |
| `rotation_turn_limit` | Proactive rotation at a configured per-session turn limit. |
| `provider_limit` | The provider ended a turn at a limit that invalidates the session. |
| `owner_rotate` | The owner explicitly asked for a rotation. |
| `owner_interrupt` | The owner cancelled, steered, or interrupted the in-flight turn. |
| `model_switch` | A model switch required a new session. |
| `membership_change` | The agent's membership in the channel changed. |
| `adapter_failure` | The adapter reported a runtime failure on the previous turn. |
| `agent_exited` | The adapter process exited. |
| `turn_timeout` | The previous turn hit an idle or hard-cap timeout. |
| `error` | Any other error path invalidated the previous session. |
| `unknown` | Cause is unknown. |

Consumers MUST treat an unrecognized value as `unknown` and keep the token
counts valid, exactly as for `stopReason`. A consumer that displays causes
SHOULD surface an unrecognized value verbatim rather than collapsing it to
`unknown`: this set extends by amendment, and a newer publisher's real teardown
reason is more useful than a shrug.

Every value except `worker_spawn` and `scope_first_turn` also names why the
**previous** session in that scope ended, so a histogram of creation causes is
also the teardown histogram — minus sessions that are still alive, and minus
those two "no predecessor" values.

### Consumer storage of decrypted payloads

A consumer that keeps a decrypted copy of a turn metric MUST store the
publisher's plaintext as received, not a re-serialization through its own
payload type. Because consumers are REQUIRED to ignore unknown fields, a round
trip through a type that predates a field silently deletes it. Where the stored
copy is append-only and the ciphertext is not retained, that deletion is
permanent and no later upgrade can recover it — and publishers are commonly
upgraded far more often than the consumers reading them.

## Publisher Behavior

- Publish exactly one event per completed turn, at turn completion, including
  turns that end in cancellation or error when usage was observed.
- Do NOT publish an event for a turn with no observed usage (all counters
  unknown); an all-null metric carries no information.
- `created_at` SHOULD equal the payload `timestamp` truncated to seconds.

## Relay Behavior

On receiving a kind 44200 event, a relay MUST:

1. Validate the event signature per NIP-01.
2. Verify `event.pubkey` equals the `agent` tag and that
   `is_agent_owner(agent, owner)` holds for the `p` tag via authenticated
   ownership lookup. Tag matching alone is insufficient.
3. Store the event durably, scoped to the owner (community-global; no channel
   scope).
4. NOT index the event in any full-text search (the ciphertext is not
   searchable and must not enter search indexes).

Reads MUST be gated: only an authenticated ([NIP-42](42.md)) reader whose
pubkey equals the `#p` tag value may receive the event. This gate applies to
**every** read path, including explicit `ids` filters — knowing an event id
MUST NOT grant access. (Some p-gated kinds exempt id-addressed lookups on the
theory that knowing the id implies authorization; kind 44200 events are
long-lived and their cleartext envelope leaks turn activity, so no such
exemption is permitted.) Unauthenticated publish or subscribe attempts MUST be
rejected with `AUTH required`; authenticated attempts from a pubkey that is not
the event owner MUST be rejected with `restricted:`.

Relays SHOULD rate-limit kind 44200 to a rate consistent with real turn
frequency (RECOMMENDED: 60 events/minute per agent pubkey).

## Client Behavior

Owners recover usage history with:

```json
{"kinds": [44200], "#p": ["<own_pubkey>"], "since": <window_start>}
```

On receiving an event, a client MUST verify the signature, decrypt with its
own secret key and `event.pubkey`, and ignore events that fail to decrypt or
parse. Clients SHOULD deduplicate by event id. For within-session ordering,
clients MUST use `(sessionId, turnSeq)` from the decrypted payload as
described above; `created_at` is suitable only for coarse time-window
queries.

## Relationship to Other NIPs

- [NIP-AO](NIP-AO.md): same agent↔owner encryption and tag scoping, but
  ephemeral and transcript-grade. NIP-AM events MUST NOT carry conversation
  content, tool calls, or protocol frames — usage numbers and identifiers only.
- [NIP-09](09.md): the authoring agent (or its owner via relay policy) may
  request deletion; relays apply standard deletion semantics.
- [NIP-40](40.md): publishers MAY set `expiration` to bound retention.

## Security Considerations

**Metadata leakage.** `p`, `agent`, and `created_at` are cleartext: a relay
operator learns that agent X completed turns for owner Y at some rate. Turn
rate is already observable from the agent's channel messages; the token
counts, cost, model, and channel remain encrypted.

**No forward secrecy.** NIP-44 does not provide forward secrecy; compromise
of the agent's private key allows decryption of captured ciphertexts.

**Integrity of accounting.** Metrics are self-reported by the agent process.
A compromised agent can under- or over-report. Owners requiring stronger
guarantees must reconcile against provider-side billing.
