# Dispatch rollout and recovery

This guide defines the deployment order and acceptance boundary for the Dispatch
product changes. Source tests and a locally built UI do not prove that a running
relay, dispatcher or executor has upgraded.

## Prepare the release

1. Record the exact base/commit, clean-tree state, all test outputs and built artifact
   hashes. Preserve a restorable copy of the current Desktop app and service binaries.
2. Run repository Rust, Desktop, web, mobile, signing and policy gates, plus the
   PostgreSQL admission/duplicate-capture/conversation-update/command-boundary tests.
   The browser suite uses a mock native/relay bridge; keep that result distinct from
   real service acceptance. Include current full Flutter tests and actual-font layouts.
3. Identify the target community, Dispatch workspace, every linked execution/capture
   channel, the durable relay public key, dispatcher public key/configuration and each
   executor's active harness version. Do not copy secret keys into a release receipt.
4. Prepare exact service configuration and reviewed binary/container artifacts.
   Schedule a maintenance window and obtain production-change approval before applying.

## Order of deployment

1. Pause the existing dispatcher through its actual host/service controls. A legacy
   service with `remote_controls` absent/false does not obey the new UI pause control.
2. Back up the database and verify its journal checkpoint/restore policy. Deploy the
   relay/admin build that supports atomic kind-40027 admission, kind-40028 reads and
   duplicate-safe conversation updates. Preserve the configured durable relay key.
   Do not create a new signing identity as a recovery shortcut.
3. Preview the legacy baseline in the configured relay community:

   ```sh
   buzz-admin dispatch-baseline --workspace WORKSPACE_UUID
   ```

   The operator environment must already provide its ordinary database/community
   configuration and `BUZZ_RELAY_PRIVATE_KEY`. Review event IDs, authors, current
   roles and the reported digest. A baseline explicitly records roles observed now;
   it cannot reconstruct historical permissions. Removed authors are recorded as
   `none`. Resolve disputed interpretations before applying. Repeat for authorized
   execution channels; review one manifest at a time (maximum 10,000 source events).

   ```sh
   buzz-admin dispatch-baseline --workspace WORKSPACE_UUID --apply-digest REVIEWED_DIGEST
   ```

   A changed manifest must be previewed again. After an uncertain response, preview
   and inspect persisted receipts before retrying. Baseline appends are irreversible
   history; do not erase signed receipts to change the interpretation. Refresh clients
   after apply because this admin operation does not broadcast old source history.
4. Upgrade and restart serving task harnesses and CLI/ACP clients to the build that
   understands task envelope v3. Prove an isolated assignment, executor update,
   independent acceptance and independent rejection against each supported harness.
   Keep production assignment disabled until this compatibility check passes. The
   relay snapshot's `taskAuthority` capability is not harness-attestation evidence.
5. Install Desktop/mobile builds and check read-only projections against actual relay
   history, including an explicitly linked task in a different authorized channel.
6. Deploy the dispatcher with `remote_controls: true`, its original persistent state
   directory, an exact service identity, host allowlist, expected heartbeat signer and
   current memberships. Validate configuration with `buzz-dispatcher check-config`.
   Begin with signed preview controls; missing controls must remain effect-free.
7. Confirm a fresh, correctly signed preview heartbeat. Review candidate explanations,
   chosen issues, import activation, executor capacity/dependencies, source audiences,
   destinations, expiry, per-cycle limit and total allowance. Activate only a small
   explicitly approved sample, then pause and inspect accepted history before expanding.

## Representative acceptance

For iOS builds, enable the reviewed `SharedSourcesPlugin` from the exact pinned
`swift-secp256k1` version through Xcode's normal trust prompt. The application
and notification extension both need the generated `libsecp256k1` module map
when importing BuzzPushKit; Debug and Release xcconfigs supply it explicitly.
Do not hide the dependency with an implementation-only import or disable package
validation to get past a native build failure.

When selecting Xcode with `DEVELOPER_DIR`, run Flutter through
`scripts/with-ios-toolchain.sh` (for example, from `mobile`,
`../scripts/with-ios-toolchain.sh flutter build ios --release --no-codesign`).
Flutter's native-asset hooks retain PATH but strip DEVELOPER_DIR, so this wrapper
keeps their xcrun calls on the same Xcode. It makes no global toolchain changes.
An unsigned build still requires the correct Apple team, app and extension
provisioning profiles, and a connected unlocked device before device acceptance.

Use disposable test work and identities on an isolated relay first. Do not wake a
production executor merely to satisfy a UI screenshot.

- Edit every human field; open two editors, create a stale conflict and reconcile it.
  Reload after a lost acknowledgement and verify one accepted change. Fail local
  storage and verify the original recovery entry is preserved.
- Read the same linked task after cold load, warm load and live continuation. Remove
  the original author's membership and verify admitted history remains stable; remove
  the viewer and verify execution details become unavailable.
- Assign with distinct owner, executor and reviewer. Confirm issue-link acceptance
  precedes the executor message. Attempt scope rebinding, self-review, outsider review,
  stale predecessor, repeat terminal transition and evidence-free review; all must fail.
- Capture the same source concurrently; observe one issue. Share the same change from
  two clients and after lost ACK; observe one conversation update. Confirm neither a
  completion notice nor a review automatically closes the issue.
- Preview, activate, pause, expire and restart automation. Change approver membership,
  audience and routing limits while a write is pending; no stale effect may escape.
  Restore an earlier database checkpoint and confirm degraded integrity stops intake.
- Inspect Board, List, Triage, My Issues, attention views, shared/personal views, project
  milestones/outcomes, decisions and delivery evidence at desktop and narrow widths.
  Use keyboard navigation and large text. Check that badges do not promote reported
  work into reviewed, merged, installed or accepted work.
- On a real simulator/device, triage an issue and resolve a decision using the existing
  identity. Change community during preparation and sending; the old signed write must
  never be redirected. Test airplane mode/lost ACK, retained drafts and accepted readback.

## Recovery

Pause effects before diagnosing a failed rollout. Preserve signed event history,
local retry journals, dispatcher reservations and the database journal; do not delete
state to refresh an allowance or re-sign an uncertain publication. Clients offer
“Check and retry” for the exact event, or “Keep as draft” for a permanently stale
change. A draft may describe a write already accepted remotely. Compare accepted
history before manually reapplying it. Reconcile an unresolved assignment link before
starting another attempt.

The CLI `track dispatch` command saves the exact signed issue link and task before
publishing either. It reads the accepted issue link back before releasing the task
message. If publication is interrupted, retry with the same `--request-id` and input;
the relay history and local recovery record determine which step remains. Recovery
records live under the CLI Tracker cache in `dispatch-assignments-v1`, scoped to the
relay, workspace and signer. Do not delete them to get past a conflict. A different
request cannot bypass an unresolved task link. If another client changed the issue
after its link was saved, inspect the latest issue, then explicitly supply that
reviewed `--if-revision` when retrying the original request. Changed assignment
content requires reconciliation instead of silently replacing the retained task.

A binary rollback does not undo appended events. An older relay will not issue new
admission receipts and an older harness may reject v3, so keep automation paused and
limit old clients to compatible operations while recovering. Record the actual
installed/service versions and test results after recovery, not just the intended
configuration.
