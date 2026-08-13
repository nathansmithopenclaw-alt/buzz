# Commitment watchdog

`commitment-watchdog.sh` detects work that was explicitly claimed and then lost
its liveness signal. It does not infer commitments from arbitrary old lock
directories or task age.

## Contract

1. When work becomes an open commitment, create a directory beneath the
   configured `watch_root` and add `.buzz-commitment.json` using
   `commitment-marker.example.json` as the schema. Write a temporary file and
   rename it into place so the timer never observes a partially written marker.
2. Put every file whose modification proves forward progress in
   `heartbeat_paths`. The worker must touch at least one declared path more
   often than `stale_after_seconds`. Long-running builds remain healthy as long
   as their wrapper updates a declared heartbeat.
3. Remove the marker when the commitment is delivered, explicitly released, or
   cancelled. Use a new `epoch` when reopening the same logical commitment.
4. Run `check` from a timer. A stale marker produces at most one escalation for
   each `commitment_id` plus `epoch` pair.

Only explicitly marked commitments are eligible. Malformed markers fail closed
and are logged; they do not produce an escalation.

For drills, leave `fallback_mentions` empty: a simulated failure must notify
only the delegator participating in the proof, never a human fallback.

## Signing and delivery

The production path reads a dedicated private key from macOS Keychain and
verifies its relay pubkey against `expected_signer_pubkey` before every send.
The signer should be an independently owner-attested watchdog identity, not the
identity whose work is being monitored. The marker validator also rejects any
delegator or fallback mention equal to the configured signer, because a
self-authored mention cannot wake that identity. A dedicated attested watchdog
identity is preferred; deployments reusing an agent identity must guarantee
that identity can never be an escalation target.

Before publishing, the watcher searches the relay for its deterministic action
token. This closes the crash window between relay acceptance and local state
persistence. A failed lookup is treated as ambiguous and blocks the send until
lookup health returns.

Dry-run receipts do not suppress a later production send for the same epoch.
They suppress only repeated dry runs.

## Operations

```bash
scripts/commitment-watchdog.sh validate --config /absolute/path/config.json
scripts/commitment-watchdog.sh status --config /absolute/path/config.json
scripts/commitment-watchdog.sh check --config /absolute/path/config.json
```

For a no-network proof, set `COMMITMENT_WATCHDOG_DRY_RUN=1`; would-be alerts are
written under `state_root/outbox`.

Run the regression suite with:

```bash
scripts/test-commitment-watchdog.sh
```
