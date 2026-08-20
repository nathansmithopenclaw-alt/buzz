---
title: "Agent management delegation v2 — grant schema, audit auto-provision, compat"
tags: [buzz, standing-delegation, upstream-adoption, design-gate]
status: active
created: 2026-08-20
---

# Agent management delegation v2

Task `7ce15b35`. Signed off (Helm, evt_20260820000812095_1b853a43). Implemented in this PR.

Source of current behavior (canonical `~/buzz` `c704af72`):

- Payload + validate: `desktop/src-tauri/src/managed_agents/global_config/mod.rs:58-104`
- Authorize: `desktop/src-tauri/src/commands/agent_management_delegation.rs:167-189`
- Sign on save: `desktop/src-tauri/src/commands/global_agent_config.rs:208-237`
- UI: `desktop/src/features/agents/ui/AgentDefaultsEditor.tsx:321-439`
- Receipts: `agent_management_delegation.rs:501-644`
- Origin: `desktop/src/features/agents/agentManagementBuffer.ts:37-58`
- Protocol: `docs/nips/NIP-SD.md`
- Standard: `GUIDES/UPSTREAM_ADOPTION_STANDARD.md`

## Current v1 (what ships today)

One owner-signed `kind:30078` stored locally on `GlobalAgentConfig.agent_management_delegation` (opaque `signedEvent` JSON; not published to the relay). Canonical payload:

```json
{
  "granteePubkey": "<lowercase 32-byte hex>",
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

Authorization is a triple: `payload.grantee_pubkey == source_pubkey` AND the capability bit AND (`!request_has_env_vars || env_vars`). `agent_config` is **not** hard-disabled — it is enforced the same as the other four bits (slice 2, main `a0a1163b`). Origin classification still requires a locally managed agent that shares the claimed channel with the owner; a payload cannot nominate a different authority.

The UI currently exposes the grantee pubkey and audit-channel UUID as text fields. That is the defect this lane removes.

## v2 grant schema

One owner-signed `kind:30078` still. Per-capability **subject**, not a second grant document and not a pubkey list that goes stale.

```json
{
  "version": 2,
  "auditChannelId": "<channel UUID>",
  "capabilities": {
    "agent_creation": { "scope": "all" },
    "agent_config": { "scope": "set", "grantees": ["<hex>", "<hex>"] },
    "env_vars": { "scope": "none" },
    "global_config": { "scope": "none" },
    "teams": { "scope": "all" }
  }
}
```

`scope` is exactly one of:

| scope | meaning |
|-------|---------|
| `none` | capability off |
| `set` | exactly the listed lowercase hex pubkeys; `grantees` required, non-empty, canonical, deduped |
| `all` | every locally-managed agent of this owner, **including agents created after this signature** |

`all` is a wildcard in the signed payload, not an enumeration taken at grant time. That is the only way later colleagues are covered without re-granting. `all` is **not** a public wildcard: origin classification still rejects anyone who is not a managed agent sharing a channel with the owner. A random pubkey cannot apply.

`env_vars` stays conjunctive: a request that carries `request.envVars` also needs the requester to match the `env_vars` subject, even when the primary capability matches.

Capability vocabulary is unchanged. Grant mutation, key export, deletion, lifecycle, relay config, and creating a new standing grant stay out of the delegated vocabulary.

### Why not enumerate-all-now

Checking every current agent and writing their pubkeys looks like "all agents" in the UI and immediately fails Nathan's forward-inclusive requirement the moment the next colleague is created. The signed `all` flag is the product, not a convenience for the picker.

### Why not an exclude list

"All except these" would keep forward-inclusivity after unchecking one name, but it is a third mode, easy to misread in receipts ("all agents" while a denylist is hiding). Unchecking one name while `all` is on **leaves `all` mode**: the capability is rewritten as `set` of the current roster minus that name. Copy in the picker states that future agents will no longer be covered until `all` is checked again.

## Authorization

```
grant_authorizes_request(grant, source, capability, has_env):
  match decode(grant):
    V1 -> source == granteePubkey
          && v1_bit(capability)
          && (!has_env || v1_bit(env_vars))
    V2 -> subject_matches(cap.scope, source)
          && (!has_env || subject_matches(env_vars.scope, source))
    other -> fail loud (do not authorize, do not draft-as-if-valid)
```

`subject_matches`:

- `none` → false
- `set` → source is in `grantees`
- `all` → true at the grant layer; origin classification still has to accept the sender as a managed agent on a shared channel

Unknown `version` (present and not `2`; v1 has no version field) fails validation of the grant itself. `get_global_agent_config` already fails the load when `grant.validate` fails — keep that. Do not treat an unknown version as "no grant" (that would silently fall back to owner-review and hide a corrupted/future grant).

## Compat and revocation

- **v1 grants keep verifying.** Decode: no `version` field + `granteePubkey: string` + boolean capability map → v1. Existing Nathan→Helm grant (kind 30078, single grantee `d590bcf4`, `agent_creation`+`teams`) continues to authorize Helm only.
- **v1 is not auto-upgraded on read.** Opening the v2 UI with a live v1 grant shows that one agent checked on the bits that are true. The next owner save of the delegation card re-signs as v2. Until that save, the on-disk signature stays v1.
- **v2 Desktop reads v1 and v2.** A v1 Desktop cannot deserialize v2 (missing `granteePubkey`, capabilities are objects). Do not write v2 until this build is the live Desktop. Rolling back after a v2 save requires restoring the previous `global-agent-config.json` or the grant field errors on load — fail-loud, not silent narrowing. We will not write a synthetic v1 `granteePubkey` alongside v2: old code would authorize only that one key and hide the rest.
- **Clearing a permission = immediate revoke.** Grant writes do **not** wait on the Agent defaults Save bar. Picker confirm, switch-off, and "Grant all permissions to all agents" persist via the existing `managed_agents_store_lock` + `prepare_delegation_signature` path against the **latest durable** config (same lock that serializes revoke vs apply, `global_agent_config.rs:137-143`). An unsaved provider/model edit in the same form is not part of that write.
- **Unchecking one agent** rewrites that capability's `set` (or converts `all` → `set` minus that agent) and re-signs immediately. Empty set becomes `none`.
- **Setting the whole grant to null** still revokes everything, as today.
- **Receipts name the acting agent.** Keep current shape: owner-signed kind:9 in the audit channel, `@Name` in the body, `p` tag = requester pubkey, capability label, `delegated` vs `owner-reviewed`, request UUID. With `all`, blast radius is larger; per-event attribution is the control. Do not add grantees as channel members just to make receipts visible to them — they already get the encrypted kind:30078 result event.

## Audit channel auto-provision

Hard requirements: receipts visible to the owner; nothing to type; no org UUIDs in the UI or in source.

Mechanism: **create-once, deterministic UUID, private stream, reuse.**

1. UUID = `uuid5(namespace, "agent-management-audit:v1:{owner_hex}:{relay_scope}")` — same pattern as starter channels (`channels.rs:571-574`), so two devices of the same owner on the same relay converge on one channel instead of minting two.
2. On first grant persist (any capability leaving `none`), `ensure_agent_management_audit_channel`:
   - If a kind:39000 with that `d` tag exists: join if needed, return its id.
   - Else create a **private** `stream` named `Agent management` with description `Audit log for delegated agent-management actions.`, no TTL, using that UUID.
3. Owner is a member as creator. **Do not auto-add grantees.** A private owner-only channel keeps the audit trail off the community roster. Nathan can add colleagues later if he wants; we will not hardcode any pubkey.
4. Store the UUID in the signed payload exactly as v1 does. Never render it. If the channel is later archived/deleted, the next grant persist recreates / re-finds by the deterministic id.
5. If create fails (offline, relay reject), the grant write fails and nothing is signed. No silent "grant with empty audit channel."

v1 grants already carry an `auditChannelId`; reuse it, do not migrate them onto the new deterministic channel.

This is org-agnostic. Production today uses a typed-in UUID (Helm + a specific channel). After v2, a fresh grant gets a private owner channel instead of `#buzz-dev`. Existing v1 grants keep their current audit channel until the owner re-saves.

## UI

Card copy (replaces "Grant one managed colleague…"):

> Grant managed colleagues narrowly scoped authority to create agents, update configuration, or manage teams. You choose who gets each permission. Clearing a permission revokes it immediately.

- Grantee pubkey field: gone.
- Audit channel UUID field: gone.
- Five capability rows remain. Collapsed summary: `Off` / `All agents` / `N agents`.
- Switching a capability **on** opens the picker; cancel leaves it off. Switching **off** revokes that capability immediately. Clicking a non-off summary reopens the picker.
- Picker, modeled on `MembersSidebar.tsx` (search shell + scrollable list), not a free-text form:
  - Search filters the owner's **managed agents** (have pubkeys) by display name. Archived identities stay out.
  - Scrollable list, checkbox in the right-hand column.
  - `All agents` master checkbox at the top (forward-inclusive `scope: all`).
  - Confirm persists.
- One top-level control on the card: **Grant all permissions to all agents**. Confirm dialog, then all five scopes = `all`, audit channel ensured, signed in one write.

Picker list is managed agents, not undeployed personas — a persona without a pubkey cannot be a grantee.

## Experiments / upstream

The card is already shipped ungated on this fork. Putting `agentManagementDelegation` in `preview-features.json` would default it **off** (manifest membership = preview; default false). That hides the live production card until someone toggles Experiments, while on-disk v1 grants would keep authorizing.

**Recommendation:** keep the card stable (not in the manifest) because this lane replaces fields on an already-shipped surface rather than introducing one. If Helm wants the experiment anyway for the upstream PR, add it and seed it ON in our rollout so Nathan does not lose the card; fresh upstream profiles stay off.

No Smith Crafts pubkeys, channel UUIDs, or hostnames in source. Protocol change ships as an update to `docs/nips/NIP-SD.md` in the same PR.

## Native surface

Apply path is already CLI (`buzz agents apply-create` / `apply-update` / `apply-global-config`). Grant mutation is owner-signed, so it stays a Desktop owner action (the UI is the human mirror of the Tauri persist, not the only door for the *operations* the grant enables). No new CLI verb required for this lane. Agents already observe authorization as apply vs durable owner-review draft.

## Tests (acceptance for the later PR)

- v1 fixture still authorizes only the original grantee.
- v2 `all` authorizes two different managed agents for the same capability; a non-managed pubkey still fails origin.
- v2 `set` authorizes listed, rejects unlisted.
- Leaving `all` by unchecking one name writes `set` minus that name; a newly created agent is then not covered.
- Unknown version fails validate loud.
- `env_vars` conjunctive on both v1 and v2.
- Audit channel created once, reused on the second grant persist; no UUID/pubkey inputs in the UI (e2e).
- Receipt `p` tag is the acting agent for an `all` grant.
- Per-agent revoke: uncheck → that agent drafts instead of applying; the other still applies.

## Coordination

- R1 (`grok/remote-control-r1` @ `9fbd497c`) does **not** touch `AgentDefaultsEditor.tsx` or the grant files. This lane is a fresh worktree off then-current main after sign-off.
- Anvil A4 nits are e2e screenshots + an upstream issue. No file collision.
- Library / Edit-in-Fleet live on R1, not in this lane.
- Sequence after sign-off: worktree + `seed-lane-target.sh`, per-worktree git identity `Grok (Smith Crafts Worker)` / `grok@smithcrafts.dev`, DCO, two-round review, `buzz pr open --channel 53f97470-aab9-43f7-87c0-8506c7208d73`.

## PR notes / mixed-version rollout

v2 write is **per-machine**. Do not save this card (and do not click Grant all) until the new build is installed on the Desktop that owns that machine's `global-agent-config.json`. A v1 Desktop cannot deserialize v2; loading a v2 file on an old build fails loud rather than silently narrowing the grant.

Both Desktops in a mixed-version window must be upgraded before either owner saves the card. Restoring a pre-v2 `global-agent-config.json` is the rollback path.
---
