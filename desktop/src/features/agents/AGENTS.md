# Agent Configuration — Contributor Rules

Scope: `desktop/src/features/agents/` (config surfaces, shared config renderer,
and the agent config core). Read this before changing how harness / provider /
model / effort configuration is modeled, rendered, persisted, or applied.

Plan of record: `Buzz/Harness-Provider-Model.md` in Morgan's Obsidian vault
(PR sequence, decisions log). PRs: #2140 (rename), #2148 (flag reduction),
#2156 (honest model states), #2158 (Agent Config Core).

## The one rule

**Harness capability facts have exactly one source: the Rust runtime catalog.**
`KnownAcpRuntime` (`desktop/src-tauri/src/managed_agents/discovery/runtime_metadata.rs`)
declares each harness's model/provider/effort env keys and capabilities. Spawn
applies them; `AcpRuntimeCatalogEntry` exposes them over IPC; and
`lib/agentConfigCore.ts` projects them into field descriptors. The frontend
never maintains a rival copy of this table. Setup guidance follows the same
rule: `requires_external_cli` is derived from `KnownAcpRuntime` and projected
to the UI rather than inferred from a runtime ID in a component.

**Second metadata source: command-keyed execution policy.**
`harness_max_parallelism` (`managed_agents/parallelism.rs`) maps the harness's
static command string to a spawn-time cap (`OPENCLAW_MAX_PARALLELISM = 5` for
OpenClaw, `PRIME_MAX_PARALLELISM = 3` for Prime Agent). This cap is not a
`KnownAcpRuntime` field because it applies to preset harnesses (like OpenClaw
and Prime) that are not in the builtin catalog. Prime also uses exclusive
session spawn in `buzz-acp` (one subprocess per channel). It is
projected onto `AcpRuntimeCatalogEntry.max_parallelism` by all four
catalog-producing constructors (builtin discovery, preset catalog, custom
discovery, custom-save response) using the **static definition command**, not
the resolved `entry.command` (which may be `null` for unavailable entries).
The frontend reads `maxParallelism` from the catalog entry and never keeps a
separate constant.

If you need a new capability fact (a new env key, a native option, a "supports
X" flag): add it to `KnownAcpRuntime` first, expose it on
`AcpRuntimeCatalogEntry`, then project it through the core. Do not shortcut
with a TypeScript lookup table or an id comparison in a component.

Model *inventory* is separate from capability metadata. Live ACP/CLI discovery
is authoritative for what an already-running harness advertises. The curated
offline inventory in `ui/agentModelCatalog.ts` supplements it with known model
IDs, display names, and family groups; its entries must be visibly marked when
they are not live-advertised. Curated Claude entries are made live-advertised
on restart by the Desktop-managed harness-wide `.claude/settings.json` catalog.
The displayed Claude union canonicalizes known alias/dated-id pairs and sorts
families as Fable, Opus, Sonnet, Haiku with newest versions first; this is a
presentation-only transform and must never remove entries from the persisted
`availableModels` allowlist.
Never infer a harness capability (env key, effort support, install state) from
that frontend inventory.

## Rules

1. **No hardcoded harness-ID checks in render code.** `runtime.id === "claude"`
   belongs in `deriveAgentConfigFieldModel` (once, with a named reason), never
   in a component. Components ask the field model what exists
   (`hasRenderableAgentConfigField`, `getRenderableEffortField`).
2. **Effort reads/writes go through the descriptor.** Use the effort
   descriptor's `currentPersistence` key, never a raw effort env literal in
   UI code. Native keys and normalization come from the runtime catalog.
   Managed-instance overrides persist the canonical `effortLevel` when native
   discovery is available; definition and global scopes use their declared
   persistence policy. Previously saved generic ACP defaults remain a fallback
   only when no canonical effort resolves, and cannot shadow a concrete pin.
3. **Field absence has a named reason, not a boolean.** Codex instance and definition edits
   expose effort through the runtime-declared `effortModelQualifier` capability:
   the control reads and writes the saved model ID's `[level]` suffix and uses
   the adapter's live native values. Local managed-instance profile edits use the
   canonical `effortLevel` override when native discovery is available. Global/onboarding surfaces keep Codex effort
   `ownedByModelId`; unsupported harnesses are `unsupportedByHarness`. Claude's
   control uses the selected model's live native values verbatim (including
   `default`); when either adapter omits effort for that model, render an explicit
   read-only explanation. New absences get new named reasons in
   `AgentConfigOmission` / `render` — never a `showX` prop.
4. **The clearing policy is the named types.** `onContextChange:
   "resetDependentValues"` (user changed harness/provider → dependent values
   reset everywhere) vs `onCatalogMismatch: "explainOnly" | "onboardingCleanup"`
   (an async catalog miss never silently erases saved state outside
   onboarding's named cleanup). Do not add mutation booleans like
   `clearInvalidModel`; extend the policy types.
5. **Fast mode follows the same capability contract.** Claude and Codex expose
   model-dependent native ACP options, persisted per agent through the
   descriptor's `BUZZ_ACP_FAST_MODE` key and applied after model selection.
   Clear it on an explicit model/runtime context change; an asynchronous
   catalog miss must only explain the mismatch. Never synthesize OpenRouter
   throughput routing as Fast mode.
6. **"Metadata unknown" ≠ "harness lacks the capability".** Passing
   `runtime: undefined` to the core means fields won't render. Surfaces must
   gate on the runtime catalog query settling (loading/error states) rather
   than letting fields silently vanish — see `AgentDefaultsEditor` /
   `DefaultConfigStep` for the pattern.
7. **One canonical behavior, disclosure presets for visibility.** Behavior
   flags were deliberately killed in #2148 (`CANONICAL_CONFIG_BEHAVIORS`).
   Surface differences are expressed via the `disclosure` preset, not new
   boolean props.  **Exception:** `onboarding-essential` hides happy-path
   helper copy (provider/model descriptions) but a non-null model-discovery
   status always bypasses the preset and renders the status line — enforced
   via `shouldShowModelStatusMessage()` (`AgentConfigFields.tsx`).
   Additionally, a successful discovery response that yields no usable options
   (`supportsSwitching:false` or empty model list) synthesizes a warning status
   via `synthesizeEmptyDiscoveryStatus()` and is intentionally **not cached**
   so that closing → reopening the dialog re-runs discovery after the user
   installs or signs into the CLI (`isCacheableDiscoveryResponse()`).
8. **Onboarding setup detects readiness; it does not select defaults.** The
   setup page derives visible and ready harnesses from the runtime catalog and
   only offers install or sign-in actions. The following defaults page is the
   sole onboarding surface that chooses `preferred_runtime`. Its complete draft
   lives in machine-onboarding session state, so Back performs no write and
   restores even incomplete edits when the user returns. Skip abandons that
   draft and advances with zero config writes. Next is the only persistence
   boundary: it consumes the shared renderer's `onValidityChange` signal,
   disables editing while awaiting `set_global_agent_config`, advances only on
   success, and leaves the draft in place with a retryable inline error on
   failure. A harness selection alone does not enable Next when the harness
   requires provider/model/credential config (e.g. buzz-agent with no
   provider). Baked build env and runtime-file config satisfy the gate. Drafts
   intentionally do not survive an app restart.
   `onboarding-agent-defaults.spec.ts` is the acceptance gate for anything
   touching this flow or the shared renderer.
9. **Omit the Model control only after a confirmed successful empty
   discovery on an optional-model harness.** When the field model marks model
   as `acpNative` (Claude Code / Codex), `shouldRenderModelControl` hides the
   picker while discovery is in flight and after IPC resolves with no usable
   options (`modelDiscoverySuccessfulEmpty` / `isSuccessfulEmptyDiscovery`).
   A thrown or unavailable discovery keeps the control so #2246 failure UI can
   render, and must not heal/clear persisted model or effort. Full disclosure
   still shows the control when Custom model is available. Required-model
   harnesses always keep the field. Gate: `defaults hides model when optional
   harness has empty discovery` (and the failed-discovery counterpart) in
   `onboarding-agent-defaults.spec.ts`.
10. **The defaults modal is progressively disclosed.** An unset global config
   starts on the Buzz Agent-first deployment fallback and carries that visible
   harness into the next saved edit. The `progressive-defaults` disclosure
   preset therefore begins at Provider for Buzz Agent, then reveals Model,
   Effort, and Advanced only after a provider is configured. Harnesses whose
   runtime metadata has no provider field skip that gate. Reveals animate their
   height through Motion and become immediate when reduced motion is requested.
   Once the Advanced toggle is visible, its expanded state is exclusively
   user-controlled: provider, harness, and required-env changes must never
   open it automatically in defaults, create, or edit flows. In Create mode,
   `Run on` belongs in Advanced directly after **Who can send instructions**;
   keep it out of the basic create fields. The defaults summary follows
   preferred-harness changes saved while the dialog is open, and its configured
   state includes required credentials as well as provider/model values. If no
   available harness can resolve, Create starts in Customize and lets unavailable
   catalog entries be selected only to expose their setup guidance; submission
   remains blocked.
   Advanced-only required credentials and incomplete remote **Run on** setup
   mark the collapsed Advanced toggle without opening it, and block incomplete
   saves.
   Runtime-file credentials satisfy Global Defaults just as they do Create and
   Edit. In Edit,
   selecting Custom command keeps its required command field beside the harness
   picker rather than hiding it in Advanced.
11. **Catalog visibility is community-scoped relay state, never a global
    definition field.** `AgentDefinition.shared` is only the active
    relay+owner projection returned to the UI. Durable heads and pending
    publications live in the scoped retention database, and explicit share
    toggles await relay acceptance before the UI claims that an agent was
    published or removed. A queued update must stay visibly queued, and the
    catalog itself must render only relay-confirmed publications — never an
    optimistic local persona.
12. **Shared agent access names the consequence where it is selected.** The
   shared respond-to field shows a persistent warning whenever `anyone` **or**
   `allowlist` is selected — both hand the host's access to someone other than
   the owner, so both disclose it and only the audience phrase differs. This
   covers persona-backed create and edit surfaces. Keep that disclosure in
   the shared field instead of adding surface-specific flags. It renders
   directly below the selector for `anyone` but *after* the people picker for
   `allowlist`, so it never sits between the user and the selection they came
   to make. The copy leads with the audience ("Anyone can use this agent to
   access…") so it reads as a warning rather than an explanation, and stays one
   sentence — don't split the mechanism into a second sentence. Both the machine
   and the stakes it names come from `lib/agentAccessWarning.ts`, keyed on an
   optional `runLocation`: instance surfaces resolve it from
   `ManagedAgent.backend` via `runLocationForBackend`, and the create flow from
   `WhereToRunDraft.runOn` via `runLocationForRunOn`. `AgentDialog` is the one
   place that resolves it for dialog surfaces and publishes it through
   `ui/AgentRunLocationContext.tsx`; the field reads that context and lets an
   explicit `runLocation` prop win. Do **not** thread the value as a prop
   through `AgentDefinitionDialog` / `AgentInstanceEditDialog` — neither uses
   the value itself, and the shared context keeps the dialog boundary stable.
   Surfaces rendered outside `AgentDialog` (e.g. `EditRespondToDialog`) pass the
   prop directly. Local names "your
   computer, including files, accounts, and connected tools"; remote names "the
   server it runs on, including any accounts and tools available there" —
   deliberately *not* the owner's files, which aren't theirs to describe on a
   host they don't own. **For a persona-linked deployed agent, the profile Edit
   dialog seeds access from the exact clicked instance and saves access through
   `update_managed_agent`; persona behavior remains the definition default, but
   must never bypass the instance command's stop, persist, publish, and restart
   boundary.** An unknown location falls back to the local wording — never hedge
   with "computer or server". A remote host requires an
   installed `buzz-backend-*` provider, and without one `WhereToRunSection`
   never renders, so "server" would name a concept the owner has never been
   shown; when it *is* remote they picked that host from the selector
   themselves. Never synthesize a run location a surface doesn't have. Don't
   expose `respond-to`, `allowlist`, Nostr, or harness jargon in primary UI
   copy. **The owner-only-access build capability is backend-independent.** When
   `getAgentAccessOwnerOnly()` is true, every managed agent's access control is
   locked to owner-only, including provider-backed agents. A provider backend
   does not prove remote execution and must never create a policy carve-out.
13. **Delegated config patches obey the same catalog and preservation rules.**
   `agent_config` and `global_config` requests resolve an `effort` value through
   the selected runtime catalog entry's `thinkingEnvVar`; they never carry a
   frontend-owned runtime-to-key table or silently fall back to a raw key.
   Global patches are overlaid onto the current durable config under the Rust
   store lock, so an in-flight query placeholder or concurrent revocation cannot
   erase unrelated defaults or reintroduce the standing grant. Existing-agent
   updates save without respawning and report that configuration changes take
   effect on next spawn. Definition behavior remains a mint-time default, so
   an existing-agent management request carries its original behavior patch to
   Rust separately: apply only the fields the request named to every linked
   instance record, preserving unrelated per-instance overrides. Each running
   `(agent, community relay)` pair keeps its old config until that pair next
   spawns. The env-only readiness exception restarts all tracked pairs only
   when every one is in setup mode; it never bounces a live non-setup pair.
   Global receipts report restart effects from the setter's returned counts,
   never from the requested field names.
14. **Shared instructions must be reviewable byte-for-byte.** Agent definitions
   execute their `system_prompt` verbatim, so catalog and snapshot review
   surfaces render the literal prompt, never the chat Markdown projection
   (which can conceal spoilers, link destinations, and image sources). Reject
   Unicode default-ignorable, bidirectional-formatting, and non-layout control
   characters at both the untrusted catalog parser and the Rust persistence /
   import boundary. Do not silently strip them: rejection keeps the reviewed
   string identical to the executed string. New sharing paths must reuse the
   same validation before they persist or activate a definition.
13. **Profile runtime sections render only reported agent data.** Missing
   runtime, model, status, command, MCP, advanced, or diagnostics values stay
   absent in every build mode. Do not fill profile or agent-panel gaps with
   development/staging examples, preview controls, or synthetic configuration;
   those values can be mistaken for the viewed agent's real configuration.
   Configuration rows show the effective value regardless of whether it came
   from an explicit choice, global default, config file, or runtime override.
   Do not add provenance lines, shadowed/struck-through values, pre-start
   placeholders, or whole-section dimming; use an em dash for an unknown value.
   Info, activity, agent-configuration, and model-setting rows use the same bare
   16px leading-icon treatment as agent management actions. Keep semantic icons
   visible in profile variants and do not wrap them in background shapes. An
   owned agent profile is entry-point invariant: opening the same deployed
   agent from Agents, a DM, or a channel must expose the same actions, tabs,
   fields, and profile-wide activity selection. Caller context may control the
   panel shell or return navigation, but must not filter or replace profile
   content. Explicit public-key targets are always exact, including stopped,
   archived, and relay-only identities. Only explicit persona navigation may
   select a representative or offer persona Start; a relay persona link cannot
   borrow a local sibling's management controls. See
   [the identity contract](../../../../docs/agent-profile-identity.md).
   Availability dots read relay presence, never a saved deployment
   receipt or runtime status. Failed/disconnected reads are unknown; lifecycle
   actions retain their separate routing. Current exact-key Online/Away presence
   suppresses Start for an inactive local record without granting Stop authority;
   list/profile/member startup guards must not interpret Offline as proof of safe
   startup. Deletion also consumes that same exact-key availability reader:
   unknown requests shutdown when a channel exists, request failure retains the
   record, and only established Offline keeps the intentional no-request path.
   Unqueried persona siblings are unknown. No presence state grants deletion or
   Stop authority; native local stop-before-remove remains independent. See
   [the availability contract](../../../../docs/agent-availability.md).
   The shared cloud marker means “Not managed on this device” only
   after ownership and successful local inventory are known. It does not imply
   hosting location, availability, or permission. Keep all identity surfaces on
   the shared provenance context, without per-row directory subscriptions. See
   [the provenance contract](../../../../docs/agent-management-provenance.md).
14. **Thinking effort has two surfaces: a local-only WRITE control and a
   read-only two-facts DISPLAY.** The write control is `EffortPickerField`
   (`ui/EffortPickerField.tsx`), a self-contained section component mounted in
   `AgentInstanceEditDialog` beside the Model block. It is **Save-gated, not
   direct-write**: the control is fully controlled by the parent dialog
   (`value`/`onChange`) and owns no mutation. The dialog persists the selection
   by embedding `effortLevel` in the locked `update_managed_agent` IPC call, so
   the effort write is atomic with any access-policy change and can never race
   or survive a Cancel or failed Save. There is no standalone
   `persistAgentEffortLevel` setter. Its gating and option compute live in the
   pure helper `ui/effortPicker.ts` (`effortPickerState`): the picker renders
   only when `agent.backend.type === "local"` **AND** a `thought_level`
   `effortConfigId` has been discovered from the running session (absent
   pre-first-session and for runtimes/models without effort support). Local-only
   is load-bearing, not cosmetic — the Rust command rejects non-local backends
   because remote effort is set at deploy time via `policy_env`. Because the
   control reads its inputs from the config surface the dialog already fetches
   (`useAgentConfigSurface`), it integrates into the dialog's existing field
   group without additional IPC. The read-only display is the `thinkingEffort`
   normalized field rendered by `AgentConfigPanel` via `NormalizedRow`, which
   already shows both facts — `field.value` (canonical, the effort the next
   spawn will launch with) and, when a running ACP session differs,
   `field.overriddenValue` struck through (the live session's current effort).
   No component owns "configured vs current" logic; the reader's canonical tier
   ordering feeds both facts. Do not add a second effort write path or restate
   the two-facts logic in a component.

15. **Fork compatibility:** **Instance effort is Save-gated.** `EffortPickerField` is controlled by
   the instance dialog or profile editor; it writes `effortLevel` through the normal locked
   `update_managed_agent` call. Cancel and failed Save keep persisted effort
   unchanged. The former standalone persistence command is removed.
   When discovered native options make this picker available, it replaces the
   legacy dialog effort field rather than appearing beside it; the Fast mode
   control stays present. A saved canonical value remains visible if discovery
   becomes unavailable. A catalog miss never renders a saved value as default.
   Remote effort still belongs to deploy-time policy and rejects local writes.
   Fork compatibility: the existing global/definition and pre-discovery effort
   paths remain until the broader upstream fold. Canonical instance effort
   takes precedence over legacy ACP effort both at spawn and session creation;
   the legacy path is used only when canonical effort is absent. Clearing the
   canonical instance override restores existing defaults, not an invented
   token budget. No static catalog or qualifier migration is implied here.

   **Live effort is a separate, session-only control.** Nathan's 2026-09-06
   plan ruling extends the former startup-only cut to existing conversations.
   Saved defaults still apply through `startup_effort` at session creation.
   The owner-authenticated `switch_effort` command names an exact channel and
   ACP session. Queue it until the current response finishes, fence the owning
   worker's next claim, then call the native `session/set_config_option` on that
   same session. Never cancel, rotate, replay a prompt, edit global config, or
   mutate the saved default to implement an effort pick. Native supported
   options and an advertised `liveEffortSwitching` capability gate the control.
   Only a matching request/channel/session/value receipt with a confirmed
   native currentValue is applied; queued, rejected and unconfirmed are distinct.
   Bound pending requests and replay receipts, invalidate stale session targets,
   and retire poisoned transport streams. Retain small configuration snapshots
   independently of the activity window; session rotation supersedes the old target.

15. **The persona `description` is public display metadata.** It is optional,
   capped at 280 characters, and validated through the shared visible-text
   policy (`validate_agent_description_text` in `definition_validation.rs`)
   on the raw authored bytes at create/update, snapshot import, publication,
   inbound sync, and the untrusted catalog parser — rejected, never stripped.
   It is deliberately EXCLUDED from `persona_content_hash`
   (`description_change_does_not_change_content_hash`), so a description-only
   edit never flips the restart badge on linked instances. Only the AUTHORED
   description exists — there is deliberately no derived/generated fallback;
   a blank description publishes an empty kind:0 `about`, exactly as before
   the field existed. Agent and team snapshots carry the authored description
   in the member profile's `about` and validate it before import. The trim/empty
   resolution exists twice and must stay in
   sync (port changes in the same PR): `lib/agentDescription.ts`
   (`effectiveAgentDescription`) feeds display surfaces, and its Rust twin
   (`managed_agents/agent_description.rs`, `effective_agent_description` /
   `record_effective_description`) feeds the publish path, where
   `profile_needs_sync` compares `about` (None == empty) so description edits
   reconcile instead of being clobbered. Persona-linked instances do not own a
   second description copy; snapshot export materializes the definition value
   only into the portable snapshot, and a dangling link resolves no description
   rather than reviving stale instance metadata. The agents-page card face always
   shows its runtime/model summary below the label, then the authored description
   as a separate clamped line when present (`FleetSection.tsx`,
   `ProjectedFleetAgentCard.tsx`, and `UndeployedPersonaCard.tsx` compose the
   data; `AgentIdentityCard` takes presentational `modelLabel` and `subtitle`
   props). The community catalog
   shows the same authored description before consent: a clamped two-line list
   subtitle for scanning and the full safely wrapped value in persona detail.
   The dialog field
   lives in `ui/AgentDescriptionField.tsx` (`AgentIdentityFields`), not
   inline in the over-1000-line dialogs.

16. **Owner-only builds constrain managed runtimes, not relay-agent mentions.**
    The compiled owner-only capability applies when Desktop starts or deploys a
    managed agent. Independently operated relay agents with NIP-OA ownership
    remain eligible in every build when their verified owner's signed
    `respond_to` policy admits the viewer and relay membership includes the
    target channel at publication. Owned nonmembers may be offered for preparation
    and Invite; this is not permission to publish. Final authorization refreshes
    the exact destination and retains captured selected identities across uploads
    and edits. Denial preserves the draft, never silently removes a selected key.
    See `docs/remote-mention-routing.md`. Marked builds require that verified owner coordinate but do
    not require it to equal the viewer; OSS builds retain compatibility with
    self-authored legacy directory records. Keep native discovery and send-time
    revalidation fail closed on invalid ownership or managed policy evidence,
    and on missing membership or directory evidence; do not add a cross-owner
    clamp to either mention path. Local `agents-data-changed` events
    refresh only local persona/team/managed-agent caches; they must never
    invalidate the remote relay directory.

17. **Databricks model discovery has one shared catalog authority.** Desktop and ACP call the shared `buzz-agent` discovery library; Desktop passes the effective merged `DATABRICKS_MODEL_FILTER` explicitly, and the library applies it to raw workspace endpoint IDs and Unity Catalog model-service FQNs after the additive union. A successful filtered-empty catalog is authoritative: it stays empty, disables switching, and never falls through to configured or known-model fallback. UC FQNs are catalog data and always use the MLflow Chat Completions route, regardless of family-looking text in their components. Global Defaults preserves the discovered model ID as the selected value while its closed trigger renders the provider-scoped display label; do not force the raw persisted ID over that label.

## Channel-only runtime controls

Desktop observer controls identify a channel, not a thread session. The harness
rejects `cancel_turn` and `switch_model` with `ambiguous_target` when that channel
has multiple known session scopes, including retained idle scopes. Do not treat
that result as success or a deferred model switch. Stop feedback waits for the
harness result matching the control type, channel, and request ID; relay delivery
alone does not prove that a turn was signalled. A missing result is unconfirmed,
not success. The activity pane must use its resolved `sessionChannelId` for
both the outgoing control and result correlation, even without a loaded
`Channel` object. Stop is unavailable in an unscoped all-channel pane.

Per-thread observer controls remain a separate protocol/UI change. Do not tell
users to type `!cancel` beside an inline mention: the owner command requires
kind 9, body exactly `!cancel` after trimming, and the agent's separate `p` tag.
The automatic-mention picker also inserts literal `@Name` into the body, so it
does not provide an exact-command workaround. The UI must state this limitation
rather than offer an ineffective command. An authorized owner can instead use
the CLI with the channel and target thread root:

```sh
buzz messages send --channel <channel-id> --reply-to <thread-root-id> \
  --mention <agent-pubkey> --content '!cancel'
```

## Standing delegation editor

Each permission exposes a labelled Choose agents button with a people icon and
keyboard focus, whether granted or off. Its switch opens the picker when
turning on and immediately revokes that permission when turning off. Picker
edits persist only on Confirm; cancellation makes no write. While saving,
selection and dismissal are disabled. Errors from the normalized Tauri Error
remain inside the active dialog with the draft intact for retry.

The native grant setter checks the audit channel using relay-signed
NIP-29 metadata and the current admin record's exact owner role. A metadata
signer is the relay, not the channel creator. Missing ownership, another owner,
archived channels, or mismatched discovery must not authorize a grant. Existing
owner-configured public audit channels retain their visibility and grant proofs;
new audit channels must be private.

## The tests that enforce this

- `agentManagement.test.mjs` — delegated patch validation, runtime-declared
  effort persistence, config preservation, truthful restart claims, and
  secret-free global review rows.
- `lib/agentConfigCore.test.mjs` — field model per harness × scope, clearing
  policy. Update when the capability model changes.
- `ui/agentConfigFieldsContract.test.mjs` — canonical behaviors + disclosure
  presets + `shouldShowModelStatusMessage` status-bypass +
  `shouldRenderModelControl` (successful-empty omit vs failure keep). If this
  fails, you probably reintroduced a per-surface flag or conflated empty with
  failed discovery.
- `ui/usePersonaModelDiscovery.test.mjs` — `synthesizeEmptyDiscoveryStatus`,
  `isCacheableDiscoveryResponse`, `deriveModelDiscoveryPending`,
  `isSuccessfulEmptyDiscovery`. If the "reopen to retry" copy becomes inert
  again, these tests will catch it.
- `ui/respondToFieldContract.test.mjs` — plain-language mode labels, the
  persistent warning contract for shared agent access, and its two render
  positions (after the people picker for `allowlist`).
- `lib/agentAccessWarning.test.mjs` — every mode × run-location copy variant
  plus both resolvers, including unknown-reads-as-local and
  blank-`runOn`-is-not-a-provider.
- `lib/personaCatalogRelay.test.mjs` and
  `ui/personaCatalogOwnerLabel.test.mjs` — reject invisible definition text
  and keep Markdown concealment syntax literal in the review surface.
- `../profile/ui/UserProfileRuntimeContent.test.mjs` — profile runtime panels
  cannot reintroduce build-mode previews or synthetic fallback controls.
- `desktop/tests/e2e/profile.spec.ts` — the owned-agent parity flow compares
  every profile tab when opened from Agents and from the agent's DM.
- `ui/AgentConfigPanelPresentation.test.mjs` — shared profile/agent config rows
  show only effective values, with an em dash for unknown values.
- `ui/effortPicker.test.mjs` — `effortPickerState` gating (local + discovered
  `effortConfigId` renders; provider backend or missing configId hides) and
  option/preselect compute, plus `effortSelectionToPersistedValue` sentinel →
  null. This is where the v4 provider regression is pinned: the write control
  must never render for a provider backend.
- `desktop/tests/e2e/onboarding-agent-defaults.spec.ts` — onboarding behavior
  acceptance coverage for readiness, failure states, defaults, session-draft
  restoration, zero-write Skip, Next save failure/retry, navigation, and
  successful-empty vs failed optional-model discovery.
- `desktop/tests/e2e/agents.spec.ts` — community catalog descriptions remain
  visible in the list and full detail before Add agent, including long
  unbroken Unicode text without horizontal overflow.
- `lib/agentDescription.test.mjs` — authored-description resolution: trim,
  blank/missing → null.
- Rust: `runtime_metadata_env_vars` tests pin spawn-time key application.
- Rust: `claude_model_catalog` tests pin comprehensive allowlisting, unknown-key
  preservation, existing-agent aliases, and custom saved-model union behavior.
- Rust: `spawn_hash` tests pin the restart-required badge for both model and
  effort changes.
- Rust: persona sharing/retention tests pin relay+owner scoping, durable
  enqueue errors, relay rejection/unavailability, and accepted publication.
- Rust: `definition_validation` and inbound persona tests pin the shared
  Unicode/control-character policy at local, import, publish, and sync gates.

## Keep this file true

**If you change how agent configuration is modeled, rendered, persisted,
applied, or cleared — update this file in the same PR.** A rule that no longer
matches the code is worse than no rule; a new pattern that isn't written down
here will be broken by the next agent that never learns it existed. Reviewers:
treat a config-behavior diff without a matching AGENTS.md diff (or an explicit
"no rules changed" note) as incomplete.

## Fork-local Subscription Account tokens

Token creation availability comes from Rust `token_supported`; components never infer it from a harness id. Token material does not cross IPC. `kind`, `minted_at`, and `expires_at` are non-secret account metadata; timestamps are UNIX seconds. The shared expiry projection rounds positive partial days up and shows Expired at zero without erasing the selected account. Settings AI owns mint, nickname and renewal; the shared selector remains in Agents Profile Runtime, Edit Agent and the DM header. Renewal is available when the creation experiment is disabled. Login accounts retain exclusive runtime admission; token accounts do not acquire a lease or reservation. See `docs/subscription-token-accounts.md` for the local storage and acceptance boundary.

Account pickers use the shared DropdownMenu radio items and Button components, including keyboard focus and portal behavior. Do not use browser-native select elements for account selection. Claude browser login creation/reconnect is retired; Rust advertises token setup only while preserving cleanup of existing login accounts. Existing selections must remain visible until explicitly migrated.

Claude token switching during active work requires an explicit preview and confirmation. Rust persists destination, generation, running intent and per-relay pause receipts before replacing a runtime. The common spawn boundary fences incomplete/failed operations; reconciliation cannot restart a captured stopped runtime. Only an explicit Start changes that intent. The broker journals active and arriving work, acknowledges cancellation cleanup, then loads the exact provider session under the selected token. Unknown cleanup or missing session references retain the hold. Operation progress is persisted and queried independently of component lifetime; each observed durable transition refreshes the managed-agent cache, including startup recovery with no picker mutation. Token authentication notices point to Retry account switch or another account, matching the rendered controls. A spawn/selection receipt does not prove provider authentication or successful resumed work. Other harnesses retain their idle-only switch gate. Default, all-current-account and removal workflows share a durable Mac-local parent operation. Its preview captures every affected agent across communities and compares saved generations, default, token mint and actual runtime intent again under the write locks. Existing bindings migrate as explicit overrides; only new local Claude agents or an explicit Use default selection enroll in the default. The selected account ID remains materialized per agent; spawn never silently substitutes a changed default. Parent progress retains completed members, fences competing changes, and exposes Retry or another destination after failure. Explicit Stops must survive preparation, restart and retargeting. Removal moves consumers or stops/detaches them before deleting credentials; a detached agent keeps its work held until a named account is selected. Default changes never switch explicit overrides. Switch-back remains explicit and quota tracking remains separate.

A generation-bound local resume acknowledgment also confirms the new broker is listening under its captured account binding. Startup recovery can run before the relay observer subscribes, so a missed lifecycle frame must not leave the account label unknown forever. A failed resume acknowledgment cannot advance readiness, and this receipt does not claim provider authentication.

The agent profile and Agents Start/Restart actions also probe the exact broker generation before confirming its running account. This readiness probe never releases an authentication hold or resumes retained work. If confirmation fails, retain the running process and expose the error with Restart available; do not label a saved selection as a confirmed runtime binding.

A detached or unfinished account operation with no saved binding renders an explicit Account required selection. Ambient must not appear selected while the durable spawn fence requires a named account.
