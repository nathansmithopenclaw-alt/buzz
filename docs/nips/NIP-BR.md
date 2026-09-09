# NIP-BR: Personal Briefing Reports

`draft` `optional`

Version-two named Summaries, sharing, and atomic run publication are specified in
[Summary v2](../summary-v2.md). Its version-two rules supersede the singleton rules
below; the original version-one format remains supported for migration.

## Abstract

This document defines a personal daily briefing protocol for Nostr
collaboration systems. A user chooses which of **their member channels**
(and which of their direct messages) feed a report. A selected reporter
agent publishes one immutable report per run. Current reporter watermarks
and the user's include-set live as addressable events so a restart resumes
from deltas, never from full history.

The protocol does not hardcode organization names, reporter names, model
providers, or channel IDs. Feature naming in a client (tab label vs
reporter display name) is a UI concern. Clients MAY ship a starter
reporter persona (name, avatar, and instructions) created during Summary
setup so a new user only chooses harness and model. That persona is not a
protocol identity and MUST NOT be required as a Welcome teammate.

## Kind allocation

| Kind | Name | Retention |
|---:|---|---|
| 30623 | `BRIEF_CONFIG` | Parameterized replaceable; keyed by `(pubkey, kind, d)` |
| 30624 | `BRIEF_WATERMARK` | Parameterized replaceable; keyed by `(pubkey, kind, d)` |
| 40030 | `BRIEF_REPORT` | Immutable published report; one event per run |

Kinds 30623 and 30624 are user/reporter-owned global state (never
channel-scoped). Kind 40030 is a regular stored event and is also
never channel-scoped.

Kinds 40020–40025 (NIP-BT), 30617–30622, and 30174–30181 are reserved
by other Buzz NIPs and MUST NOT be reused.

## Discovery

All three kinds MUST carry exactly one discovery tag
`["t", "buzz-brief"]`.

Reports and config SHOULD also carry `["p", "<owner-hex>"]`. When a
reporter pubkey is known, they SHOULD also carry
`["p", "<reporter-hex>"]`. Implementations that later p-gate these
kinds can do so without rewriting historical envelopes.

Relays MUST `#p`-gate kinds 30623, 30624, and 40030: a REQ that can
match any of those kinds is closed unless the filter's `#p` values
exactly equal the authenticated reader. Relays MUST also exclude those
kinds from NIP-50 `search_tsv` so digest bodies are not full-text
searchable. Clients MUST still treat reports as personal: they MUST
NOT render kind 40030 in public Pulse, Home, or channel timelines.

## Config (30623)

Addressable coordinate: `(owner-pubkey, 30623, "summary")`. `d` MUST
be exactly `summary`. One live config per owner.

Content is strict JSON:

```json
{
  "v": 1,
  "reporter_pubkey": "<64-hex or null>",
  "hour": 9,
  "minute": 0,
  "timezone": "America/New_York",
  "auto_include": true,
  "channels": ["<uuid>", "..."],
  "dms": ["<uuid>", "..."],
  "exclude": ["<uuid>", "..."]
}
```

Rules:

- `hour` is 0–23. `minute` is 0–59. Together they are local wall time
  in `timezone`, not UTC.
- `timezone` is an IANA name or `UTC`.
- `channels` and `dms` are include sets. Only conversations where the
  **owner is a member** are valid; a community catalog MUST NOT be
  used as the universe.
- `exclude` is the opt-out set used when `auto_include` is true.
- `auto_include` true: newly joined member conversations are included
  until the owner unchecks them. Archived (`archived_at`) or deleted
  conversations always leave the effective set.
- Direct messages MAY appear in `dms`. Adding the reporter as a visible
  third DM participant is the wrong shape; DMs ride a read-scope grant
  (native) or are omitted from bot-membership (v1).
- `read_delegation` (optional, default false) explicitly permits the selected
  reporter to request bounded read-only transcript snapshots from the owner's
  selected conversations. It never confers membership or write permission.
  See [Summary read delegation](../summary-read-delegation.md) for the current
  HTTP bridge extension, revocation, and rollout boundary.
- Unknown fields are invalid. Duplicate singleton tags are invalid.

Matching tags:

```text
["d", "summary"]
["t", "buzz-brief"]
["p", "<owner-hex>"]
["p", "<reporter-hex>"]   // when reporter_pubkey is set
["auto-include", "true|false"]
["tz", "<IANA>"]
["hour", "<0-23>"]
["minute", "<0-59>"]
```

Each included conversation projects as a repeatable
`["include", "channel|dm", "<uuid>"]` tag. Each exclusion projects as
`["exclude", "<uuid>"]`.

## Watermark (30624)

Reporter-authored. `d` MUST be `wm:<channel-uuid>`. Content:

```json
{
  "v": 1,
  "channel_id": "<uuid>",
  "last_event_id": "<64-hex>",
  "last_created_at": 1710000000
}
```

The model only ever summarizes deltas since this watermark. Missing
watermark means "no prior run for this conversation" — not "read all
history".

## Report (40030)

Immutable. Content:

```json
{
  "v": 1,
  "run_id": "<uuid-v4>",
  "title": "Wednesday, 27 August",
  "markdown": "Human-readable report",
  "status": {},
  "period": { "since": 1710000000, "until": 1710086400 },
  "sources": { "channels": ["<uuid>"], "dms": ["<uuid>"] }
}
```

`markdown` is the human rendering. `status` is a JSON object the
orchestrator (or any agent) can query; unknown keys are preserved.
`run_id` matches the `d` tag.

Tags:

```text
["d", "<run-id>"]
["t", "buzz-brief"]
["p", "<owner-hex>"]
["title", "<title>"]
["published_at", "<unix-seconds>"]
```

History folds recency-first by `(created_at, event_id)`. There is no
replaceable head: a later run is a new event.

## Channel universe

The include universe is **the owner's current memberships**, not every
channel in the community. That is the upstream-correct shape for a
workspace that contains many human teammates who do not share every
room.

Legacy v1 setup added the reporter as a `bot`-role member on included
channels. Current setup uses the explicit `read_delegation` grant for both
channels and DMs and does not change source memberships. Older memberships
remain ordinary memberships until separately removed.

## Scheduling

The daily fire uses the existing workflow engine. A schedule trigger
MAY carry `timezone` (IANA). Cron fields then mean local time in that
zone, including DST. Omitting `timezone` preserves today's UTC
behavior (`0 9 * * *` is 09:00 UTC).

The briefing config is the user-facing source of truth for "09:00
where I am." Saving the config MUST create or update a companion
scheduled workflow in the owner/reporter DM with the same hour, minute, and timezone. Clearing
the include set or disabling Summary MUST delete that workflow.

On-demand: mentioning the reporter is sufficient to request a run.
The reporter still writes a kind 40030 report; it MUST NOT rely on
Inbox (`mention` / `needs_action` / `activity`) as its source.

## Client behavior

Clients that implement a Summary surface:

1. Gate the feature behind the workspace Experiments toggle.
2. First open with no config: a builder listing the owner's member
   channels and DMs, all on, auto-include on, local time default
   09:00. Save publishes kind 30623.
3. Later: the latest report, a recency-first history, and a Sources
   control that reopens the same builder.
4. Auto-drop archived and deleted memberships from the displayed set
   even if they remain in the stored include list.

## Relay behavior

- Ingest kind 30623 / 30624 / 40030 as global-only (ignore stray `h`
  for `channel_id` routing).
- Kind 30623 / 30624 follow ordinary NIP-33 replacement.
- Kind 40030 is stored immutably. NIP-09 may redact a report.
- Kinds 30623 / 30624 / 40030 are `#p`-gated (owner + reporter) and
  MUST NOT be indexed for NIP-50 search.
- Kind 40030 ingest MUST reject the event unless `event.pubkey` equals
  `reporter_pubkey` on the owner's live kind 30623 (`d=summary`).
- Unknown kinds remain rejected.
