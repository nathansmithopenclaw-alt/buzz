NIP-ME
======

Memory Envelopes and Truth Receipts
-----------------------------------

`draft` `optional`

This NIP defines a provider-neutral shared-memory capability for a Nostr
workspace. Memory is native at the protocol and client surfaces, while truth
authority remains behind an implementation-defined provider. A relay event is
never authoritative merely because it exists.

## Read plane

Implementations expose authenticated `POST /memory/v1/query` and
`POST /memory/v1/status`. Requests use NIP-98 with a payload hash. The HTTP
Host binds the community before any tenant-scoped read. The relay MUST enforce
workspace membership and derive the caller's accessible channel set itself.

Every request MUST name exactly one output context:

```json
{ "kind": "channel", "id": "<uuid>" }
{ "kind": "dm", "id": "<uuid>" }
{ "kind": "workspace_public" }
```

Context-less requests and channel/DM contexts outside the relay-derived access
set MUST be rejected. Memory has two privacy grades: `workspace_public` and
`restricted`. Effective clearance is the lower of caller clearance and output
context clearance. Guests MUST NOT receive restricted records.

If a read cache is used, its authorization identity MUST contain at least
`(pubkey, community, output context, policy id, provider id, generation)`.
Query mode, terms, slot, and limit are additional cache-key members, never
substitutes for that authorization identity.

## Query

`/memory/v1/query` accepts `mode` as `current`, `why`, or `history`; an optional
exact `slot`; an optional full-text `query`; and a bounded `limit`. Provider
reads have a hard deployment-configured timeout and no retries.

A provider timeout or failure MUST produce an explicit response with
`unavailable: true`, no records, and a Truth Receipt whose status is
`unavailable`. It MUST NOT be returned as an ordinary empty result. Memory
availability MUST NOT block session creation or turn execution.

## Truth Receipt

Every result and every response carries a receipt with:

- status: `current`, `proposed`, `superseded`, `stale`, `disputed`, `expired`,
  `unknown`, or `unavailable`;
- exact slot and `current_winner_for_slot` when asserting current;
- generation high-water and generation health;
- authority and evidence hash when present;
- provider id and health;
- policy id.

A client MUST render green `current` only when provider health and generation
are healthy and `current_winner_for_slot` exactly equals the returned record id
for the receipt's non-empty slot. Deployments MUST keep a separate current-
rendering gate disabled until that provider's oracle semantics have passed
independent conformance verification.

## Provider contract

The provider receives only relay-derived identity and access context. It owns
storage, append-only history, supersession, and truth computation. Buzz's
reference crate exposes object-safe asynchronous `status` and `query` methods.
The bundled naive provider is deliberately non-governance: deterministic
per-slot LWW, explicit supersession links, token full-text search, and monotonic
generation. Governance engines remain external implementations of the same
trait.

## Future event kinds

The write plane is outside this stage. The design reserves `40200` for immutable
memory proposals, `40201` for publisher-signed status receipts, `40202` for
generation checkpoints, and `40203` for member-signed actions. None of those
events makes current truth authoritative on the relay.
