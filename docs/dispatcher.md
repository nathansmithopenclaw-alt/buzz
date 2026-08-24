# Buzz Triage Dispatcher — Operations Guide

`buzz-dispatcher` is the resident automation service for Buzz Tracker triage
(task c6d2bd9d, plan `PLANS/TRACKER/P2_DISPATCHER_PLAN.md`). It projects the
tracker stream locally and converts queue state into deterministic actions:

- **Capture** — 📌 reactions in monitored channels become `triage` issues.
- **Routing** — label-table auto-routing via the dispatch path
  (task-envelope v2 + kind-40023 sharing one request id).
- **Escalation** — threshold ladder with at-most-once firing per
  `(issue, threshold, qualifying-interval epoch)`.
- **Liveness** — kind-40024 heartbeat every tick; an external watchdog
  surfaces stale services via channel message only.

All decisions are deterministic (no LLM judgment in the rule path), and all
effects are idempotent via UUIDv5 request ids plus a fired-request ledger.

## Subcommands

| Command | Purpose |
|---|---|
| `buzz-dispatcher check-config --config <path>` | Load + validate config; print a redacted summary. No identity required. |
| `buzz-dispatcher run --config <path>` | Resident capture/route/escalate/heartbeat loop. |
| `buzz-dispatcher watchdog --config <path>` | External heartbeat watcher. Runs under its OWN identity (separate launchd job). |

## Identity (env, never files)

The service key comes from the environment — the same seam `buzz-cli`
documents. No key material ever lives in the repo or the config file.

| Variable | Required | Meaning |
|---|---|---|
| `BUZZ_PRIVATE_KEY` | yes (run/watchdog) | Service key, hex or nsec. |
| `BUZZ_AUTH_TAG` | no | NIP-OA auth tag JSON, sent as `x-auth-tag` when present. |
| `BUZZ_RELAY_URL` | fallback | Used when `relay_url` is omitted from config. |

## Config schema (strict JSON, unknown fields rejected)

```jsonc
{
  "relay_url": "https://relay.example",        // or omit + set BUZZ_RELAY_URL
  "service_id": "triage-dispatcher",           // slug; heartbeat d-tag
  "stream_id": "<tracker stream UUID v4>",
  "poll_interval_secs": 30,                    // 1..=3600
  "monitored_channels": ["<uuid>", ...],       // pin-capture channels
  "pin_emoji": "📌",
  "default_channel": "<uuid>",                 // fallback notify/route channel
  "routing": [
    { "label": "area:payments", "owner": "<64-hex pubkey>", "channel": "<uuid|null>" }
  ],
  "thresholds": [
    {
      "id": "unassigned_24h",                  // STABLE — part of request id
      "condition": { "kind": "unassigned" },   // or unassigned_with_label {label}
                                                // or deadline_passed
      "window_secs": 86400,
      "action": "notify_home"                  // or digest | decision_surface
    }
  ],
  "roles": { "<64-hex pubkey>": "maintainer" }, // fold authorization
  "decision_surface": { "channel": "<uuid>" },
  "liveness": {
    "channel": "<uuid>",
    "owner": "<64-hex pubkey|null>",
    "stale_after_secs": 300
  },
  "state_dir": "/absolute/path/outside/repo"   // BINDING: never repo-relative
}
```

`roles` maps workspace pubkeys to `member`/`maintainer` for fold
authorization. The dispatcher's own service pubkey is merged automatically
with at least `member` so its capture roots fold valid; grant it
`maintainer` here when it needs terminal/reopen authority (M13).

## State directory (binding amendment 3)

`state_dir` must be **absolute and outside the repo checkout** (validated at
load). It holds three atomically-written files:

| File | Contents |
|---|---|
| `events.jsonl` | Append-only journal of every ingested relay event. |
| `cursor.json` | Composite `(created_at, event_id)` high-water cursor. |
| `requests.json` | Fired dispatch request ids (at-most-once ledger). |

A corrupt journal or cursor is a **hard error** — the dispatcher never
silently resets state. Refresh is inclusive (`since = cursor.created_at`)
with event-id dedupe, so restarts resume with no gaps or duplicate effects.

## Rule semantics

1. **Capture.** A `<pin_emoji>` reaction in a monitored channel plans one
   triage issue per pinned message: `request_id = UUIDv5(ns, "pin:"+event)`,
   `source_event_id` set, home channel = the reaction's channel. Re-pins and
   duplicate reactions collapse on the request id + a `source_event_id`
   pre-check. **Un-pinning is a NO-OP (binding amendment 1):** the ledger is
   append-only; reaction removal never deletes or transitions a captured
   issue. Do not "clean up" the triage queue by un-pinning.
2. **Triage arrival vs routing (exclusive).** An unassigned triage/todo issue
   matching a routing label is dispatched via envelope-v2 + kind-40023
   (M18 dispatch path only) — and that **preempts** the arrival notice
   (Helm 2026-08-24: the envelope is channel-visible and carries the owner
   wake, so a separate notice would be a second ping for the same fact).
   Exclusively UNROUTED triage arrivals get one arrival notice per triage
   entry: the "a human needs to triage this" signal.
3. **Escalation ladder.** Thresholds carry stable config ids. Firing is
   at-most-once per `(issue, threshold, epoch)` where
   `epoch = floor(entry_time / window_secs)`; `entry_time` is
   `entered_status_at` for unassigned conditions and the due value for
   `deadline_passed`, so a genuine re-entry legitimately refires.
4. **Digest batching (R4).** `digest` escalations coalesce into one message
   per channel per tick; each still records its own kind-40023 action.
5. **Import suppression (M19).** Issues whose root or any revision carries
   import provenance never trigger any rule.

Every emission re-checks the fired ledger before submitting and marks it after
an accepted relay receipt, so replays stay at-most-once.

## Liveness and watchdog (M17, amendment 2)

`run` publishes a kind-40024 heartbeat every tick with
`service_id`, `instance_id`, `observed_at`, the composite `cursor`, and
`health`. The `watchdog` subcommand runs under a **separate identity**
(different `BUZZ_PRIVATE_KEY`, separate launchd job): it reads the newest
heartbeat per `(stream, signer, service_id)` and, when the configured
`service_id` is silent past `stale_after_secs`, posts **one channel message**
per stale episode to the liveness channel (mentioning `liveness.owner` when
set). Any signer for the service counts, so a service-key rotation does not
blind the watchdog. **The surface is message-only — never a shared-canvas
mutation.** A dead process cannot author its own death notice; watchdog death
is covered by the layer above (disk-guard pattern).

## Deploy-time checklist (Helm/Nathan — not done by this crate)

1. Mint the service nsec (Nathan-gated) and inject it via
   `BUZZ_PRIVATE_KEY` in the launchd environment.
2. Add the service pubkey to the workspace roster and grant
   `TrackerRole::Maintainer` (roles config) if terminal/reopen authority is
   needed; add it to monitored channels + the tracker stream.
3. Author the config file with routing table, thresholds, monitored channels,
   decision surface, liveness owner, and an out-of-repo `state_dir`.
4. Validate with `check-config`, then install the launchd jobs
   (`docs/launchd/com.smithcrafts.buzz-dispatcher.plist` and
   `docs/launchd/com.smithcrafts.buzz-dispatcher-watchdog.plist`), each with
   its own identity.
5. Live-proof on the isolated stack (relay build ≥ `b18d7b07` per amendment 4):
   pin → one issue; re-pin ×3 → still one; routed issue → envelope + 40023
   same request id; SIGKILL mid-run → restart resumes from cursor with no
   duplicate actions; dispatcher stopped past the window → exactly one
   watchdog mention.

## Protocol impact

None new on the wire. The dispatcher completes adoption of kinds 40023/40024
exactly per NIP-BT (content fields, tags, idempotency keys). Any gap found
during build becomes NIP-BT errata in the same PR (UPSTREAM_STANDARD §5).
