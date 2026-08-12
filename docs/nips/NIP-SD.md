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

`agent_config` covers updates to an existing personal agent. `global_config`
covers a narrow patch to global agent defaults. Neither capability permits key
export, deletion, lifecycle controls, relay configuration, grant mutation, or
creation of a new standing grant.

## Owner-signed grant

Desktop stores a grant containing this canonical camel-case payload:

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

The payload is serialized as the content of a `kind:30078` event
signed by the current Desktop owner. Desktop treats that event as an opaque
authorization proof: its signature, author, kind, content, grantee, audit
channel, and capability bits must all validate against current state, and the
decoded signed payload must exactly equal the stored payload.

Changing any grant field causes Desktop to sign a new proof. Setting the grant
to `null` revokes it immediately. Capability bits are checked independently;
granting `agent_config` does not grant `global_config`, and vice versa.

Any request containing `request.envVars` additionally requires `env_vars`, even
when its primary capability is `agent_config`, `global_config`, or
`agent_creation`. Reserved Buzz identity and control variables are rejected
before a side effect. Public receipts include key names and value hashes, never
raw values.

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

Desktop validates the live owner-signed grant and atomically records a new
authorized request as `applying` before any management side effect. A UUID is an
idempotency key: replaying identical content returns the recorded disposition;
reusing the UUID with a different sender, capability, or payload is rejected.

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
