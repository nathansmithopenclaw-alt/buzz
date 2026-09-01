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

## Promotion plane

The write plane is admission-first: only typed promotion-candidates ever enter the
review queue. `fact_proposal`, corrections and supersessions of promoted facts, and
`verified_working` claims are queue-eligible. Log-grade types (`observation`,
`work_item`, `artifact`, `build_report`, `open_question`) cannot render `current`,
never queue, and never request human attention. A record that merely corroborates the current winner of
its slot attaches as evidence and MUST NOT queue, because it cannot flip the slot.

Promotion is always a deliberate, signed act. No timer, threshold, or policy executor
mints `current`: there is no `auto` or `lazy` promotion mode in this protocol, not
even as a schema value. Automation is expressed only by routing `review` to a
delegated agent key holding a `memory_promotion` capability grant (see Delegated
review verdicts), so every automated promotion carries the same signed act and audit
trail a human promotion does.

## Lifecycle

The durable state set is exactly: `proposed` (ingested, not yet routed), `review`
(routed for a decision — mandatory review or a pre-promotion dispute), `awaiting_human`
(escalated by a delegated reviewer; durable until a human decides), `promoted`,
`rejected`, and `expired`. `disputed` is not a durable state. A pre-promotion dispute
moves the candidate to `review`; it does not affect the record. A post-promotion
dispute removes the receipt's green `current` presentation for that slot and opens the
governed supersession or reaffirmation flow; the promoted event is never deleted or
mutated — it stays `promoted` — and the chain `proposed → promoted → disputed →
superseded/reaffirmed` stays visible.

Dispute honesty is protocol, not configuration: a member's self-dispute never resets a
graduation window or any dispute counter, and repeat disputes on a reaffirmed slot
rate-limit hard — dispute-spam is a denial of service on the queue itself. Per-member
dispute quotas, where a deployment sets them, are workspace configuration.

`review`-tier proposals lapse to `expired` after 30 days; the event is retained.
Log-grade records do not pend and do not expire. An ignored escalation also lapses to
`expired` and never promotes. Neither lapse is configurable.

## Memory actions (40203)

Member decisions are `kind:40203 MemoryAction` events: member-signed, encrypted to
their audience, immutable. The action set is `approve`, `reject`, `dispute`,
`withdraw`; `escalate` is valid only from an actor presenting a valid delegated
`memory_promotion` grant. Every action references the `kind:40200` proposal id, the
proposal payload hash, and the community/provider audience, and carries an
idempotency key. Effectiveness is by durable ingest order, not render order.
Receipts (`kind:40201`) are publisher-signed output and MUST NOT carry member
commands.

## Delegated review verdicts

A reviewer that is an agent promotes only under a valid NIP-SD `memory_promotion`
capability (a v3 keyed singular proof; see NIP-SD). Authority lives in the engine,
not in the agent's instructions: the engine authenticates the reviewer, resolves the
frozen policy, validates the typed proof bundle, and mints the governed promotion
event itself. Provenance on the promoted record names both `verified_by` (the
reviewer key) and `executed_by` (`memory_promotion_engine`). The reviewer never
signs a `current_fact`, never holds the publisher key, and never receives a generic
provider write path.

Normative requirements:

- **promoter ≠ proposer.** The engine MUST reject any promotion where the approving
  key equals the proposing key. An actor's own approval never satisfies review.
- **Generation binding.** An `approve` is bound to the engine generation it was
  evaluated at. If the generation advanced within the claim's dependency set between
  evaluation and execution, the promotion is rejected to re-verification at head —
  not queued, not warned.
- **Verify the rendered claim.** The promoted statement is derived from what the
  evidence establishes and verified as it will render in the slot; a gap between the
  proposer's sentence and the evidence's statement is itself an escalation.
- **Typed proof bundle.** The proof is producer-bound: `bound_verifier` equals the
  promoter, the evidence is signed by the reviewer after admission, and the payload
  hash is bound to the referenced 40200. Copied or proposer-supplied evidence, and
  bare `content.verification` assertions, MUST reject.
- **Human fallback is configured, never inferred.** The escalation destination is an
  explicit allowlist frozen into the policy at admission. A valid NIP-OA attestation
  tag disqualifies an entry; role, kind-0, and a missing auth tag prove nothing. The
  grants, not the policy, are the source of truth: the backing NIP-SD v3 keyed proofs
  carry `humanFallbackAllowlist` values that agree byte-for-byte, rotation is an
  atomic set re-sign (see NIP-SD), and admission copies that agreed value into the
  frozen policy snapshot for the item. A later allowlist rotation does not reroute
  in-flight items, consistent with policy changes never applying retroactively. Any
  grant disagreement fails closed to human review.
- **Escalation is engine state, not a speech act.** The absence of a valid proof
  means the item never leaves `review` — the safety default is structural. The
  explicit `escalate` action is enrichment: it carries reason codes and the recorded
  attempt, and transitions the item to durable `awaiting_human` with a recorded
  assignment. Escalations count separately in the workspace digest.

### Validator rule identifiers

On any typed rejection the validator names exactly one of these identifiers. The
conformance fixtures ([`NIP-ME.promotion-fixtures.json`](NIP-ME.promotion-fixtures.json))
name the rules each negative case must fire, so an implementation cannot pass by
rejecting for an unrelated reason — the same device as NIP-MP's named ingest rules.

| Identifier | Fires when |
| --- | --- |
| `promoter-not-proposer` | the approving key equals the proposing key |
| `generation-binding` | the engine generation advanced within the claim's dependency set between evaluation and execution |
| `proof-producer-bound` | the evidence was not produced and signed by the reviewer after admission (copied or proposer-supplied) |
| `proof-after-admission` | the evidence signature predates the proposal's admission time |
| `bound-verifier-equals-promoter` | the bundle's `bound_verifier` is not the promoting key |
| `admission-routing-resolves` | routing resolved an empty reviewer set at admission |
| `unknown-shape-loud` | a grant's version/shape combination is not a defined document (see NIP-SD) |
| `memory-promotion-not-boolean-true` | `memory_promotion` was granted as bare `true` instead of a typed object |
| `claim-types-in-vocabulary` | `claimTypes` includes a value outside the delegable vocabulary |
| `claim-types-non-empty` | `claimTypes` is empty |
| `allowlist-only-with-grant` | `humanFallbackAllowlist` is present on a proof that does not grant `memory_promotion` |

These identifiers cover typed rejections only. Fail-closed outcomes that authorize
nothing (`no_promotion_authorized`) and structural defaults that never leave `review`
(`remains_review`) are not rejections and carry no rule identifier.

## Promotion policy and receipts

Promotion policy is versioned governance, not config lookup. Each revision is an
engine-ledger object: `policy_id`, content hash, signer/authority, `effective_from`,
supersession link. Resolution happens once at admission and is stamped on the
proposal and its receipts. Policy evaluation is engine-side normative; clients MUST
NOT gate on locally-held policy. Promotion validation is a typed proof validator with
its own conformance fixtures
([`NIP-ME.promotion-fixtures.json`](NIP-ME.promotion-fixtures.json)) — never a
generic boolean authority predicate.

Every promotion receipt carries the policy id and hash, the evaluated rule, and
three audit labels: `authority ∈ {human_reviewed, delegated_agent}`;
`verification_class ∈ {reproduced_by_promoter, asserted_by_proposer,
none_registered}`; `reproducible_by ∈ {any_member, privileged, not_reproducible}`.
`none_registered` renders as a distinct non-green "believed, unverified" state —
never green `current`.

## Declarative rule schema

The review-routing rule is a single declarative value:

```json
{
  "match": {
    "actor_class": "<class>",
    "envelope_type": "<type>",
    "criticality": "<grade>",
    "writer_history": "<class>"
  },
  "action": "review"
}
```

`review` is the only action the schema carries; routing destinations (which human or
which delegated reviewer key) are workspace configuration, not protocol. The
following are mandatory `review` and non-configurable: anything that would render
green `current`; corrections and supersessions of promoted slots; `verified_working`
claims; identity-mapping create/rotate/revoke; promotion-policy writes;
restricted-realm facts; anything `disputed`; publisher/journal/genesis operations;
first-time or unmapped writers. Policy changes never apply retroactively to
in-flight items.

Where routing applies, admission MUST resolve a non-empty reviewer set for the item:
a rule that fires against a configured destination resolving to nobody is a loud
rejection at admission (`admission-routing-resolves`), and the queue never holds an
item nobody owns. Routed review items p-tag the designated reviewer, so the item
reaches that reviewer's inbox through the existing mention-join.

## Unconfigured default

A deployment with no workspace policy MUST apply the distributed default: any member
may promote from the queue; first-time writers (no promoted-without-dispute history)
always route to `review` and graduate automatically after K = 3 undisputed
promotions. The queue is a view — an inbox filter and a CLI listing with identical
rows — and the digest is product, not protocol: at most one replaceable item per
person.

## Event kinds

Memory uses four dedicated kinds. No event in this NIP makes current truth
authoritative on the relay: the relay stores and transports; only the engine's
governed promotion event, minted after a valid typed proof, promotes.

### 40200 MemoryProposal

Member-signed, immutable, `h`-scoped to the community. The content is NIP-44-
encrypted to the community's ingestion identity; statement text is never public.
Public tags carry only protocol, engine, and request identifiers plus the payload
hash — nothing else is readable without the ingestion key.

The envelope type is a closed set of six, carried inside the encrypted payload:
`observation`, `work_item`, `artifact`, `build_report`, `fact_proposal`,
`open_question`. The set is closed: an unknown envelope type is a loud ingest
rejection, not an unparsed novelty.

The encrypted payload names its own community, and ingest MUST reject a proposal
whose payload community disagrees with its `h` tag. Because the NIP-44 recipient is
this community's ingestion identity, the same bytes replayed against another
community fail at decrypt or at the community check — cross-community replay is
rejected at ingest, never at read.

### 40201 MemoryReceipt

Publisher-signed output of the deployment's publisher identity. The status set is
closed: `accepted_proposal`, `rejected`, `promoted`, `superseded`, `expired`. A
receipt conveys status, never authority: it is emitted after the fact, is never an
input to a promotion decision, and MUST NOT carry member commands. It references the
proposal id, carries the engine event id and generation high-water it reflects plus
the redacted proof hash, and MUST NOT carry statement text into mixed channels — a
reader without memory access learns that something happened, never what.

### 40202 MemoryCheckpoint

Publisher-signed binding of engine generation and transaction position to the relay
journal: generation, tx, journal epoch, journal sequence, and journal tail hash. The
latest valid checkpoint in durable ingest order defines the generation binding in
force; older checkpoints remain as audit trail. Read-plane generation health and the
green-`current` gate derive from checkpoints — a read served without a current
checkpoint cannot render green `current`.

### 40203 MemoryAction

Member-signed actions are defined in [Memory actions](#memory-actions-40203) above.

A structural alternative — generalizing the WF-08 approval gate (kinds 46010–46012)
to typed subjects including `memory_promotion`, instead of a memory-only action pair
— remains open and is decided at implementation-lane cut. It changes where the kind
numbers above live, not any invariant in this NIP.
