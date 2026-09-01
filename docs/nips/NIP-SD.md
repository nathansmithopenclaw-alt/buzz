# NIP-SD: Standing Delegation for Agent Management

`draft` `optional`

## Abstract

This document defines Buzz's owner-signed standing delegation for management
requests made by an agent the owner controls. It lets an owner grant narrow,
independently revocable capabilities while preserving the existing owner-review
path for every ungranted or revoked request.

The current capability vocabulary is:

- `agent_creation`
- `agent_config`
- `global_config`
- `env_vars`
- `teams`
- `memory_promotion`

`agent_config` covers updates to an existing personal agent. `global_config`
covers a narrow patch to global agent defaults. `memory_promotion` covers
casting delegated review verdicts on memory promotion candidates, limited to
the claim types the grant names. No capability permits key export, deletion,
lifecycle controls, relay configuration, grant mutation, promotion-policy
writes, or creation of a new standing grant.

## Owner-signed grant

Each grant contains a canonical camel-case payload as the content of a
`kind:30078` event signed by the current Desktop owner. Desktop treats every
event as an opaque authorization proof: its signature, author, kind, content,
and decoded payload must all validate against current state, and the decoded
signed payload must exactly equal the stored payload.

`global-agent-config.json` has two compatible storage fields:

- `agent_management_delegation` is the legacy single-proof slot. Existing
  files load without modification, and adding another grantee MUST NOT
  re-sign or rewrite this proof.
- `agent_management_delegations` is an object keyed by canonical lowercase
  grantee pubkey. Every value is an independently signed singular grant whose
  signed `granteePubkey` MUST equal its object key.

Changing one keyed grant signs only that grantee's new proof — except
`humanFallbackAllowlist`, whose rotation is a set operation (see v3 payload).
Removing one
key revokes only that grantee; other proof bytes stay unchanged. Clearing the
whole delegation surface removes both fields. Capabilities are checked
independently; granting `agent_config` does not grant `global_config`, and vice
versa.

### v1 payload

A grant with no `version` field is v1: one grantee and boolean bits.

```json
{
  "granteePubkey": "<lowercase 32-byte hex pubkey>",
  "auditChannelId": "<channel UUID>",
  "capabilities": {
    "agent_config": true,
    "global_config": false,
    "agent_creation": false,
    "env_vars": false,
    "teams": false
  }
}
```

v1 grants keep verifying. Desktop does not auto-upgrade them on read. New
keyed entries deliberately reuse this singular payload, so multi-grantee
storage does not require a new signature format. Version and slot are now
coupled, not orthogonal:

| Slot | Valid payloads |
| --- | --- |
| legacy `agent_management_delegation` | v1 singular (no `version` field), v2 aggregate (`version: 2`, compatibility) |
| keyed `agent_management_delegations` | v1-shape singular (no `version` field), v3 singular (`version: 3`) |

A v2 aggregate is defined only in the legacy slot; a versioned proof is
defined only as a keyed singular. The next owner save re-signs as the current
build's version: v2-era builds write keyed entries as v1-shape and re-sign
the legacy slot as v2; v3 builds write keyed entries as `version: 3`.

### Unknown versions

A grant whose `version` field is present and not `2` or `3` fails validation
loud. It is not treated as "no grant" and must not fall back to owner-review. A
v1 grant has no `version` field. A known version in the wrong shape fails the
same way: a `version: 3` proof in the aggregate shape is an unknown-shape
grant (see v3 payload). Desktop refuses to load a `global-agent-config.json` that
cannot deserialize for this reason; restoring a pre-v2 copy of that file
is the rollback path.

### v2 aggregate payload (compatibility)

Earlier Desktop builds wrote one aggregate v2 proof with a per-capability
subject. Current Desktop continues to validate and authorize an existing v2
proof from the legacy field, but new writes create independently signed keyed
v1 proofs instead.

```json
{
  "version": 2,
  "auditChannelId": "<channel UUID>",
  "capabilities": {
    "agent_creation": { "scope": "all" },
    "agent_config": { "scope": "set", "grantees": ["<hex>"] },
    "env_vars": { "scope": "none" },
    "global_config": { "scope": "none" },
    "teams": { "scope": "all" }
  }
}
```

`scope` is exactly one of `none`, `set`, or `all`. `all` is a wildcard in the
signed payload: later locally-managed agents of this owner are covered without
re-granting. It is not a public wildcard — origin classification still requires
a locally managed agent sharing a channel with the owner.

For `scope: "set"`, `grantees` is required and must be canonical lowercase
32-byte hex pubkeys, deduplicated and sorted. An empty list is stored as
`none`, not as `set` with zero entries. Implementations MUST reject a signed
grant whose `set` entries are non-canonical, duplicated, or empty.

When the owner edits an aggregate v2 proof, Desktop expands its current
effective managed-agent roster into singular keyed proofs. `all` therefore
means all managed agents at conversion time; future agents remain ungranted
until the owner adds them. No deployed v1 proof is re-signed by this conversion
unless that grantee's effective capabilities changed.

The audit channel is created-once as a private stream named `Agent management`
with a deterministic UUID. The namespace is the same UUID used for starter
channels:

```
namespace = 3ce33bea-8f09-5f1b-9c85-8a7d2659e6b0
name      = "agent-management-audit:v1:{owner_hex}:{relay_scope}"
id        = uuid5(namespace, name)
```

`owner_hex` is the Desktop owner's lowercase 32-byte hex pubkey.
`relay_scope` is the canonical websocket identity of the workspace relay:

- `http` and `ws` fold to `ws`; every other scheme (`https`, `wss`, unknown)
  folds to `wss`. A literal scheme comparison of `https://` vs `wss://` MUST
  NOT produce two channels.
- host is lowercased
- default ports are omitted (`80` for `ws`, `443` for `wss`)
- path, query, fragment, and trailing slash are dropped

Two devices of the same owner on the same relay therefore converge. Existing
v1 grants keep their current audit channel until re-saved. Grantees are not
auto-added as members.

If a `kind:39000` already exists for that UUID, Desktop reuses it only when
`metadata.pubkey` equals the current Desktop owner. Otherwise grant persist
fails closed and nothing is signed — the UUID inputs are public, so a
pre-created channel on a multi-tenant relay is otherwise a squat. Desktop
must not join that channel or post receipts into it.

A v1 Desktop cannot deserialize a legacy v2 proof. Older builds ignore the new
keyed field and therefore cannot administer those grants; do not save global
agent defaults from an older build after keyed grants exist, because an old
serializer can drop fields it does not know.

A request that carries `request.envVars` is authorized only when the requester
matches the `env_vars` subject (the same subject-match as the primary
capability), even when that primary capability is `agent_config`,
`global_config`, or `agent_creation`. Checking that `env_vars` is not `none`
is not sufficient: `scope: "set"` must still contain this source, and
`scope: "all"` still requires origin classification. Reserved Buzz identity
and control variables are rejected before a side effect. Public receipts
include key names and value hashes, never raw values.

### v3 payload

v3 adds the `memory_promotion` capability and the fail-closed state a grant
carrying it must establish. v3 is defined only as a keyed singular proof: it
reuses the keyed v1 singular shape — one `granteePubkey` that MUST equal its
storage key, one `auditChannelId`, boolean bits for the five capabilities
Desktop consumes on the request path — plus `version: 3`, the
`memory_promotion` capability, and `humanFallbackAllowlist`. Canonicalization
and the audit channel are unchanged.

```json
{
  "version": 3,
  "granteePubkey": "<lowercase 32-byte hex pubkey>",
  "auditChannelId": "<channel UUID>",
  "humanFallbackAllowlist": ["<hex>"],
  "capabilities": {
    "agent_config": false,
    "global_config": false,
    "env_vars": false,
    "teams": false,
    "agent_creation": false,
    "memory_promotion": { "claimTypes": ["fact_proposal", "verified_working"] }
  }
}
```

The storage key is the grantee, so `memory_promotion` carries no subject of
its own. It is either `false` or absent (not granted) or an object whose
`claimTypes` grants this one grantee; a boolean `true` fails validation loud,
because granting without naming claim types is not a shape its author can have
meant. The five boolean capabilities keep their keyed v1 meaning and matching
rules exactly — the same request-path checks, including the `env_vars`
request-shape check, and the same Desktop-side consumption.

`memory_promotion` authorizes its grantee to cast delegated review verdicts —
approve, reject, or escalate — on memory promotion candidates whose claim type
is listed in `claimTypes`. It conveys permission to act under the governing
memory policy; it does not convey authority over what becomes canonical truth.
Unlike the five boolean capabilities, which Desktop itself consumes on the
request path, `memory_promotion` is checked by the consuming promotion
engine: the engine MUST re-validate the live owner-signed grant in its own
process at use time, so revocation takes effect at the engine's next check
rather than at the next client save. A grant evaluated only in the client is
a suggestion.

`claimTypes` is required whenever `memory_promotion` is granted, and MUST be a
non-empty, deduplicated, sorted subset of the delegable claim type vocabulary:

- `fact_proposal`
- `verified_working`

Claim types outside this vocabulary are not delegable: corrections and
supersessions of promoted facts, restricted-realm facts, identity-mapping
changes, and anything a workspace policy marks always-human are reviewed by a
human regardless of any grant. A grant naming a claim type outside the
vocabulary fails validation loud rather than narrowing to the known subset —
the same lesson as the unknown-version rule, because a reader that silently
ignored an unrecognized claim type would widen the grant to claim types its
author never saw.

`humanFallbackAllowlist` is required whenever `memory_promotion` is granted,
and MUST NOT be carried by a proof that does not grant it. It names the
pubkeys an escalated review falls back to when the delegated reviewer cannot
verify, and MUST be a non-empty, deduplicated, sorted list of canonical
lowercase 32-byte hex pubkeys. It lives in the signed grant because it cannot
be inferred: the absence of an agent attestation proves nothing about who is a
person, and a fallback set that cannot be established is a delegation whose
failure mode is an unowned review queue.

The allowlist is owner-wide, not per-grantee. Each keyed proof carries its own
copy because every keyed proof is independently signed and must validate
standalone inside the promotion engine's process; the copy is not a per-grantee
value. Every keyed proof that grants `memory_promotion` MUST carry the same
allowlist, byte-for-byte in canonical serialization. Grant persist fails
closed when the allowlist cannot be established — the delegation is not
signed — when a newly added promotion-granting proof would disagree with an
existing one, and when an allowlist rotation would not produce a fully
agreeing set. Rotating the allowlist with two or more promotion-granting
grantees cannot be done proof-by-proof: the first re-signed proof would
disagree with the rest and be refused. An owner save that changes the
allowlist therefore re-signs ALL `memory_promotion`-granting keyed proofs as
one atomic set write, and the agreement check is evaluated on the resulting
set, not proof-by-proof. Disagreeing proofs found at validation time (for
example, from a restored backup or a partial write) fail closed to human
review: no promotion is authorized by
any of them. There is no owner-level allowlist slot; byte-for-byte agreement
across independently signed proofs is what produces one owner-wide value
without adding a second write path.

No capability, `memory_promotion` included, permits writing the memory
promotion policy. Policy writes are the rules a delegated reviewer would
otherwise be judged by: a grantee that could edit its own review rules could
widen its grant's effect without ever touching the grant. Promotion-policy
changes remain owner-reviewed regardless of any standing grant.

v3 exists only in this keyed singular shape. A proof that carries
`version: 3` in the v2 aggregate shape — per-capability subjects and no
`granteePubkey` — is an unknown-shape grant and fails validation loud, the
same as an unknown version: the version number is known, but the signed
document is not the grant this specification defines, and silently re-reading
it as keyed would authorize something its author did not sign. The aggregate
shape remains valid only for v2, which stays compatibility-only in the legacy
slot.

A v2 Desktop cannot deserialize v3. Do not write v3 until this build is
installed on the machine that owns `global-agent-config.json`.

## Request transport and identity

The grantee signs an encrypted owner-scoped observer frame (`kind:24200`). Its
decrypted payload has this envelope:

```json
{
  "type": "agent_management_request",
  "action": "update",
  "requestId": "<UUID>",
  "intent": "apply",
  "request": {
    "channelId": "<shared channel UUID>",
    "agentName": "Bolt",
    "systemPrompt": "New instructions"
  }
}
```

Desktop accepts requests only from a locally managed agent that shares the
originating channel with the owner. The sender signature identifies the
grantee; a payload cannot nominate a different authority.

## Existing-agent update

`action: "update"` requires the `agent_config` capability unless the patch is
strictly `envVars`, in which case the primary capability is `env_vars`.
Supported patch fields are:

- `displayName`
- `systemPrompt`
- `runtime`
- `provider`
- `model`
- `effort`
- `respondTo` and `respondToAllowlist`
- `parallelism`
- `namePool`
- `envVars`

The target is resolved by the current display name and must be one editable
personal definition. The patch overlays only supplied fields. Allowlist
pubkeys are canonicalized by the CLI and validated by the existing persona
save boundary.

An update saves the definition through `update_managed_agent` semantics. For
behavior fields, Desktop also applies only the fields named in the request to
every currently linked instance record; unrelated instance overrides are
preserved. This is required because the spawn-time inbound-author gate reads
the instance record, while definition behavior remains a mint-time default.

An agent-config update does not stop or respawn a running agent. Each running
`(agent, community relay)` pair therefore keeps its prior config until that pair
next spawns, and the result and audit receipt state that configuration changes
take effect on the next spawn. The narrow `envVars` readiness transition is the
only exception: when every tracked pair for a linked local agent is in setup
mode and the patch makes it ready, Desktop restarts all of those tracked pairs
on their original relays. It never cycles an already-ready, non-setup pair.

## Global defaults update

`action: "global_config"` requires `global_config`. Supported patch fields are:

- `provider`
- `model`
- `preferredRuntime`
- `effort`
- `envVars`

Grant settings, automatic-approval settings, owner credentials, and Claude
account selection are deliberately not part of this request vocabulary.
Desktop overlays the patch onto the current durable global config while holding
the Rust store lock. Every field not named by the request is preserved, and a
concurrent revocation cannot be overwritten by a stale frontend snapshot.

Effort is a logical value, not a hard-coded environment key. Desktop resolves
the selected or preferred runtime through the Rust runtime catalog and writes
the catalog entry's `thinkingEnvVar`. A runtime without that capability rejects
the effort patch.

The existing global-config setter may restart a running local agent only when
its effective spawn environment changes or readiness changes from not-ready to
ready. Non-effective changes cause zero respawns. Results and audit receipts
report the setter's actual restart and restart-failure counts.

## Durable classification, retries, and fallback

Desktop first resolves the request signer to its exact keyed grant. If no keyed
entry exists, it checks the legacy v1/v2 proof. A present but invalid keyed
entry fails closed and MUST NOT fall through to broader legacy authority.
Desktop atomically records a new authorized request as `applying` before any
management side effect. A UUID is an idempotency key: replaying identical
content returns the recorded disposition; reusing the UUID with a different
sender, capability, or payload is rejected.

If `intent` is `draft`, or no current matching grant exists, Desktop records the
request as `pending_draft`. Pending drafts retain FIFO presentation order. The
owner may approve one request without creating or widening a grant. Revocation
therefore changes later `apply` requests into the same durable owner-review
fallback rather than discarding them.

Terminal results are retained and returned as owner-signed encrypted
`kind:30078` events addressed to the grantee. Interrupted applies fail closed on
restart; Desktop does not blindly repeat a non-idempotent side effect.

## Audit receipts

Every terminal apply publishes a `kind:9` receipt to the grant's audit channel.
The receipt is signed by the same Desktop owner identity that signed the grant,
mentions the requesting grantee, labels the capability and authority
(`delegated` or `owner-reviewed`), names the outcome, and includes the request
UUID. Signed receipt bytes are retained before publication so retry publishes
the same event ID.

Audit publication is complete only when relay acceptance matches the retained
receipt's event ID. An unrelated NIP-42 AUTH acknowledgement cannot satisfy this
check.

## CLI examples

```bash
# Instructions/config update. Reuse the UUID only for an identical retry.
buzz agents apply-update \
  --request-id 70fc630c-a0ca-42e8-9392-326c5d4ef9c1 \
  --channel 7c07e659-3610-42f4-9a5e-1e9973c09da9 \
  --agent-name Bolt \
  --system-prompt - < instructions.md

# Narrow audience plus runtime configuration.
buzz agents apply-update \
  --channel 7c07e659-3610-42f4-9a5e-1e9973c09da9 \
  --agent-name Bolt \
  --respond-to allowlist \
  --respond-to-allowlist <hex-or-npub> \
  --model gpt-5 \
  --effort high \
  --parallelism 4

# Global default effort. No env_vars grant is needed because effort is a
# typed global_config field resolved through runtime metadata.
buzz agents apply-global-config \
  --channel 7c07e659-3610-42f4-9a5e-1e9973c09da9 \
  --preferred-runtime codex \
  --effort high
```
