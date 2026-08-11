# Linear-to-Tracker migration

The migration is an explicit three-phase operation. It never fetches Linear
data itself:

```bash
pnpm tracker:migrate plan \
  --input .scratch/linear-full-export.json \
  --workspace <tracker-stream-uuid> \
  --state-map PLANS/TRACKER/LINEAR_STATE_MAP.json \
  --user-map PLANS/TRACKER/LINEAR_USER_MAP.json \
  --project-map PLANS/TRACKER/LINEAR_PROJECT_MAP.json \
  --pilot-map PLANS/TRACKER/LINEAR_PILOT_MAP.json \
  --report OUTBOX/LINEAR_TRACKER_MIGRATION_PLAN.json

pnpm tracker:migrate apply <same flags> \
  --confirm-plan-sha256 <plan-sha256> \
  --checkpoint .scratch/linear-tracker-checkpoint.json

pnpm tracker:migrate verify <same flags> \
  --report OUTBOX/LINEAR_TRACKER_MIGRATION_VERIFY.json
```

`apply --resume` reuses a checkpoint only when its plan hash matches. Network
exit code 2 is retried with bounded backoff; schema, authorization, and
conflict failures stop immediately. Every imported issue, revision, relation,
link, and comment carries deterministic UUIDv5 request identity and typed
Linear provenance. Tracker watchers mark those events
`dispatcher_suppressed: true`.

The export must set `complete_export: true` only after all source pages are
present. Plan fails closed on incomplete extraction, duplicate issue IDs,
unmapped states/users/projects, missing relation targets, or invalid pilot
maps. Its report includes source counts by state, team, project, and lifecycle.

Map values use these shapes:

```json
{
  "state-map": {"<linear-state-id>": "in_progress"},
  "user-map": {"<linear-user-id>": {"pubkey": "<64-hex>"}},
  "project-map": {
    "<linear-project-id>": {
      "project_id": "<uuid>",
      "name": "Dispatch",
      "slug": "dispatch",
      "statuses": ["triage", "backlog", "todo", "in_progress", "in_review", "done", "cancelled"],
      "default_channel_id": "<uuid>",
      "default_verify": "Exact-head verification"
    }
  },
  "pilot-map": {
    "<linear-issue-id>": {
      "event_id": "<full-64-hex-nip34-event-id>",
      "labels": ["priority:P1", "due:2026-08-20", "project:dispatch", "linear:<linear-issue-id>"]
    }
  }
}
```

The repository fixtures under `scripts/tracker-fixtures/` exercise these
shapes; they are test data, not production mappings. Production map files and
the complete export are operator-reviewed inputs and must not be inferred.

Verify reads every imported issue with comments and proves exact equality for
source IDs, field values, import provenance, comments, canonical relations,
typed links, projects, pilot links, and total counts. Any unexpected Tracker
record is a mismatch rather than silently ignored.

## Project limitations

Tracker-managed projects must be signed by the workspace signer and carry the
immutable `buzz-project-id` tag. Slug uniqueness cannot be enforced by a relay
across unrelated signers. Cross-signer upstream NIP-MP projects without the
tag are referenceable by coordinate with a warning, but cannot be updated by
`buzz track project`.

Upstream candidates are a standardized immutable project-ID tag and a
standard rename/move marker for addressable NIP-MP projects. Until those
exist, Tracker records rename history in kind 40025 and deletes the old
kind-30621 address with NIP-09.
