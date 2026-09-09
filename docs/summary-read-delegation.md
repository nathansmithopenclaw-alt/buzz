# Summary setup and read-only delegation

Summary setup uses three steps: Agent, Conversations, and Schedule and review.
Nectar uses the saved preferred runtime and inherited model/provider defaults.
Its controls share Edit Agent's model discovery, curated/live model merge,
PersonaDropdownField, and model-dependent effort and fast-mode descriptors.
An existing agent opens the normal instance editor; those edits apply to that
agent's other work as well. Choosing a reporter alone never rewrites it.

The managed-agent picker is shared with Settings agent management delegation.
Both consume `useManagedAgentsQuery`. Summary selects one agent; permission
editing selects a set. Cancel abandons the picker draft.

## Authorization

Saving Summary explicitly sets `read_delegation: true` in the owner's signed
kind 30623 `d=summary` configuration. Older configs omit this flag and grant
nothing. Reads require the exact current reporter, an explicitly selected
channel/DM, no matching exclusion, current owner membership, and a live room.
The owner can revoke access by removing the source, changing the reporter,
clearing `read_delegation`, deleting the config, or leaving the room.

The authenticated NIP-98 `/query` bridge accepts a single snapshot filter:

```json
[{"summary_owner":"<owner hex>","#h":["<conversation UUID>"],"since":0,"limit":100}]
```

Only `summary_owner`, one `#h`, `since`, `until`, `before_id`, and `limit` are
accepted. Limits are 1–200. Results contain kind 9/40002 messages and 45001/45003 forum posts/comments,
never other private event kinds. There are no live subscriptions, membership
mutations, write grants, search/count grants, or cross-community reads.
The config and owner membership are read from the writer before fetching and
checked again before returning. A changed grant invalidates that page.

The response is `{"summary_read":1,"events":[...]}`. The CLI rejects an older
relay's ordinary array response rather than misrepresenting unrelated results
as a delegated transcript. Agents use:

```sh
buzz briefs config get --owner OWNER
buzz briefs read --owner OWNER --channel CONVERSATION --since WATERMARK --limit 200
```

For additional pages use the oldest returned timestamp as `--until` and its
id as `--before-id`. Watermarks advance only for successfully read sources.
The signed source selection is the grant; changing local checkboxes alone has
no effect. Auto-include updates selected IDs when the user saves the current
conversation list; it does not create an unbounded read grant.

## Schedule and migration boundary

The daily workflow is hosted in the owner's private DM with the reporter,
including when all selected sources are DMs. Its prompt tells the agent to read
the owner config and use the bounded CLI read path. Source membership is no
longer added or removed by Summary setup. Pre-existing memberships from older
Summary versions remain ordinary memberships; no provenance exists to safely
remove a membership that may also serve other work.

This changes the local Summary protocol and needs coordinated relay, CLI, and
Desktop rollout. It does not establish upstream allocation of the existing
NIP-BR event kinds or prove live provider execution. Save failures remain visible
and retryable; creation of the agent/DM/workflow and signed config publication
are distinct operations, not one distributed transaction.

## Scheduled and manual runs

Summary setup stores an optional `schedule` alongside the existing v1 source
configuration. Missing `schedule` retains the legacy daily `hour`/`minute` in the
saved IANA timezone. A schedule is `{ "on": "schedule", "cron": "…" }` or
`{ "on": "schedule", "interval": "…" }`; `null` selects manual-only mode and
removes the companion workflow without revoking the saved source selection.
Saving zero sources still removes the workflow and revokes read access.

The schedule step reuses Workflow scheduling fields, with the Summary's saved
local timezone. It supports 15/30-minute and hourly intervals, daily, selected
weekdays, monthly, and custom cron (including lists of hours for several runs per
day). Invalid schedule drafts cannot be saved. Sequential config saves use a
monotonically increasing event timestamp so rapid schedule edits do not lose to
NIP-33 same-second ordering.

**Run summary** reads the current saved config, starts the selected managed agent
if stopped, and sends one explicit-pubkey request in the owner's DM with that
agent. Startup, DM creation and message signing are bound to the captured relay
and owner identity. This action does not edit the schedule or read grant. The UI
shows request acceptance separately from report completion, links to the agent
conversation, and refreshes reports for up to ten minutes. Concurrent button
clicks are suppressed; uncertain delivery is shown and never automatically
retried. Both scheduled and manual prompts summarize new activity from existing
watermarks, without inventing work or repeating earlier material.
