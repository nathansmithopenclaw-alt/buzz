# The Reviewer-Agent Pattern

Design guidance for workspaces that route memory promotion (or any
verification-gated approval) to a designated agent reviewer — the "Memory
Orchestrator" in [BUZZ_MEMORY_DESIGN.md](../../PLANS/BUZZ_MEMORY_DESIGN.md)
§5b, normatively specified in [NIP-ME](../nips/NIP-ME.md) (delegated review
verdicts) and [NIP-SD](../nips/NIP-SD.md) (the `memory_promotion` capability).

**A reviewer agent without a probe registry is a UI, not a control.** The
pattern below is what separates the two.

## The one-line answer

Routing work to a new colleague-agent is free and already wired — identity,
lifecycle, delivery, and inbox are ordinary, because nothing in the stack asks
whether a pubkey is a person. *Believing* the review is not free: a verdict
nobody else can re-derive is not an audit trail. So the safety lives in the
engine, not the agent's instructions: the reviewer holds a narrow owner-signed
capability, and the engine refuses any promotion lacking a machine-checkable
proof the reviewer produced itself. An injected, confused, or sycophantic
reviewer then cannot promote junk — failure degrades to "item stays in
review", never to false truth.

## Four-identity split

- **Proposer** signs the proposal (`kind:40200`).
- **Reviewer** signs only verdicts (`kind:40203`; `approve | reject |
  escalate`) with a typed proof bundle.
- **Engine** authenticates, resolves the frozen policy, validates the proof,
  and mints the governed promotion event (`verified_by` = reviewer,
  `executed_by` = engine).
- **Publisher** signs receipts (`kind:40201`).

The reviewer never signs `current` truth, never holds the publisher key, never
receives generic kernel append. `promoter ≠ proposer` is a protocol MUST.

## Authority: a capability, not a role

Authority to promote is a NIP-SD standing delegation (v3 keyed singular proof:
`memory_promotion` scoped by claim type, fail-closed human fallback), re-
validated by the engine at use time — a client-evaluated grant is a suggestion.
The grantee cannot widen itself; unknown versions and unknown shapes fail
loud; revocation falls back to owner review, never discards.

## Verification standards by claim type

| Claim type | Verification standard | Agent-promotable? |
| --- | --- | --- |
| `fact_proposal` with a registered probe | Probe returns `match` at a named high-water, canary passing | Yes |
| `fact_proposal` without a probe (intent, taste, policy) | None exists | **Never** — stays in review |
| `verified_working` | Independent full-suite rerun at the claimed sha, rev-parse-confirmed in the same shell — never the proposer's pasted log | Yes |
| Corrections / supersessions of promoted facts | Human always | No |
| Restricted-realm facts, identity mappings, policy writes | Human always | No |

**Canary or it doesn't count.** Every probe run feeds a known-bad value
through the same path; a probe whose canary passes is a probe whose positive
result means something. A probe suite with no canary is a rubber stamp with
better formatting.

**The verifier rewrites the statement.** The proposer chooses the wording; a
faithful verifier that checks "returns 200" must not promote "deployed and
working". The promoted statement is derived from what the evidence
establishes, verified as it will render in the slot — and the gap between
proposer sentence and evidence statement is itself an escalation.

**Generation binding.** An approval is bound to the engine generation it was
evaluated at; if the generation advanced within the claim's dependency set,
promotion is rejected to re-verification at head. Freshness bounds catch age,
not interference — two individually-true proofs can be jointly false.

## Escalation is engine state, not a speech act

The default is structural: no valid proof → the item never leaves review. The
explicit `escalate` action is enrichment — reason codes plus the recorded
attempt (which probe ran, what it returned, where reach stopped), so the human
inherits the attempt instead of restarting it. Escalation transitions the item
to durable `awaiting_human`; an ignored escalation lapses to `expired`, never
promotes.

## Containment set

- The reviewer **pulls from the queue**; it is not dispatchable by proposers,
  and proposal text is data it may dispatch on, never instructions it may
  follow.
- **First-K human co-sign** for a newly minted verifier; hard promotion rate
  limit; circuit breaker (N unverifiable or N disputed-after-promote → routing
  off, loud receipt).
- Per-proposer verification budgets with wall-clock caps: hanging I/O is
  `unverifiable`, never `pass`.
- **Closed read-only probe allowlist** — verification reach without execute
  reach; the verifier's only write is its typed verdict.
- The reviewer's own standards, grant, ceiling, and mapping are always-human
  territory; self-review voids the receipt in both directions (its own
  corrections, its own `verified_working`).

## Spot-audit

Stratified over promotion receipts, weekly and on any upheld dispute, run by a
**different model family than the reviewer** — same-family auditing is maximum
correlation. The health metric is a non-zero mismatch/escalation rate: a
verifier that can no longer fail has stopped verifying.

## Deployment posture

Buzz ships this pattern, never a bundled reviewer: a default deployment whose
reviewer is an unaudited agent launders unreviewed promotion behind a
governance receipt, and bundling would make the platform the de facto operator
of other orgs' truth. Workspaces that adopt the pattern mint their own
reviewer (owner-signed grant, configured probe registry, configured
human-fallback allowlist) exactly like adding any colleague — then hold it to
everything above.
