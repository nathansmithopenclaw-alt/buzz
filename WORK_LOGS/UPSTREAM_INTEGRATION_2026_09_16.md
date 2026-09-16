# Upstream integration, 2026-09-16

This merges the 43 upstream commits through `213092074` into the Smith Crafts
fork. It is a source integration. App installation, adapter upgrades, relay and
push-gateway deployment, mobile signing, and live-provider acceptance are separate.

## Pinned inputs

| Role | Commit |
| --- | --- |
| Fork, first parent | `960cd5064e4d7a9fb4ffd2a7cca303b986a4c5ae` |
| Upstream, second parent | `213092074c269bee90c87163672f9c2b8d714cfe` |
| Common ancestor | `3c7f288c60d67df78577b237e27c3dfc8831aaa1` |

The fork was 455 commits ahead and 43 behind by ancestry. Upstream changed 582
paths; 147 paths changed on both sides. The initial merge had 76 conflicted paths
and 159 conflict blocks. Counts include tests, fixtures, documentation, and merges;
they are not counts of independent features.

Integration worktree: `/Users/titusclaw/.buzz/WORKTREES/upstream-integration-20260916`.
Branch: `codex/upstream-integration-20260916`. Canonical local main remained at the
first parent throughout resolution and validation. No GitHub push is part of this
operation: `origin` points to upstream, not to our canonical fork.

## Resolution record

- **ACP ownership and scheduling:** combined upstream owner generations and held
  scope deadlines with the fork's exclusive-worker scheduling, account quiescence,
  pending provider-session closure, session-creation attribution, live effort,
  and publication receipts. Returning a stale worker still invalidates and queues
  closure of its provider session. Dispatch keeps the fork's single preclaim.
  A hold timer is armed only when that scope has a claimable worker; an unrelated
  idle exclusive worker must not cause an expired-timer spin. Added a regression
  binding that decision to the production claimability helper.
- **Context and replay:** retained fork delivery/publication evidence while adding
  upstream exact-event, per-session delivered-context accounting. Targeted,
  capacity-paced overflow recovery replaces the overlapping broad resubscribe
  path. Claude background-task support remains.
- **Conversation policy:** added definition/instance `session_policy` alongside
  the fork's avatar, subscription-account, residency, and effort fields. Actual
  local and remote launch plus prospective snapshots resolve linked-definition
  inheritance through the same policy helper. Imports, catalogs, teams, delegated
  edits, and restart semantics carry the new setting. Retired the old global
  thread-session experiment; retained unrelated fork experiments. See the required
  installation transition below.
- **Fleet, Library, and identity UI:** preserved the fork's layout and profile
  activity behavior, then applied npub display and squircle avatars within it.
  `AddMemberSearchResultRow.tsx` remains deleted; its display intent is applied in
  the replacement members sidebar. The fork-only delegation card now uses npub.
  Updated two incoming browser fixtures to assert the retained Fleet structure.
  Removed the obsolete global-thread-toggle browser test and its matcher.
- **Database history:** renamed upstream relay migration 0045 to
  `0051_retain_push_revocation_tombstones.sql`. The existing fork migrations 0001
  through 0050 are byte-for-byte unchanged. Added a real PostgreSQL upgrade test
  that preserves old ledger checksums, retains revocation tombstones, admits a
  replacement live installation, rejects duplicate live keys, and tolerates a
  repeated migration run. Push-gateway migration 0005 remains separate.
- **Dependencies:** both Cargo locks resolve rustls 0.23.45, retaining fork-only
  dependencies. Integrated the MeshLLM upgrade and the `buzz-pi-acp` source
  transition. The Pi command override remains reserved because it controls child
  execution. Codex's source adapter floor is 1.10.0; no installed adapter changed.
- **Git default branch:** integrated upstream route/CLI/CAS behavior with the
  fork's trusted public-scheme authentication contract. The router extension
  supplies the trusted scheme; direct-router tests fall back to configured relay
  scheme, matching existing Smart HTTP behavior.
- **Fixture reconciliation:** updated ownership generations, policy fields,
  tokio clocks, flush attempts, connection public scheme, and mesh recovery
  fixtures. Removed duplicated inline test helpers where the fork had already
  extracted them. These changes do not remove fork runtime behavior.

## Validation-discovered repairs

The isolated PostgreSQL desired-state setup omitted `pgcrypto` even though the
schema declares it. A journal insert failed with missing `digest` on both the
candidate and the pinned fork baseline. Setup now explicitly installs that
extension into its run-owned template database.

Two older test assumptions conflicted with the fork's event journal: moderation
cleanup attempted to delete a referenced community, and a row-lock regression
held serial inserts open while they waited on the per-community journal lock.
The former now relies on the per-test database teardown. The latter holds a
channel lock compatible with foreign-key reads and proves eight concurrent event
inserts/commits finish without needing a channel update lock. Production journal
semantics are unchanged. A reporter-notice concurrency test similarly delivered
and deleted a warm-up event, then tried to insert the same journaled event again.
It now pre-creates only the DM channel before the two actual deliveries; the
assertions still require exactly one notice and the current claim token. Six
journal PostgreSQL tests were also outside the
runner's discovery convention; renaming their test module makes them discoverable.
The reporter-notice failure was reproduced at the pinned fork baseline with its
own desired-state schema and the declared pgcrypto prerequisite
(`test-reporter-baseline-2.log`: expected 1 notice, observed 0), and the repaired
candidate passed (`test-reporter-final.log`). The earlier baseline attempt used
an empty database and is not evidence for this comparison.

## Validation and evidence

Evidence directory:
`/Users/titusclaw/.buzz/RESEARCH/upstream-assessment-20260916/`.
The original `UPSTREAM_MERGE_ASSESSMENT.md` includes all 43 commits and their PRs;
`evidence.json`, `merge-tree.txt`, and `conflict-hunks.txt` retain the input and
conflict inventory. The assessment describes its earlier read-only phase; this
record describes the subsequently authorized integration.

| Check | Result | Evidence |
| --- | --- | --- |
| Full repository `just ci` | Passed, exit 0 | `ci-6.log` |
| ACP suite within CI | 1,116 passed, 4 ignored | `ci-6.log` |
| Desktop UI unit tests | 6,786 passed | `ci-6.log`, `test-frontend-2.log` |
| Native desktop library | 3,508 passed, 20 ignored; terminal/integration suites also passed | `ci-6.log` |
| Flutter main suite | 2,232 passed, 4 skipped | `ci-6.log` |
| Flutter without configured push gateway | 3 passed | `ci-6.log` |
| Isolated PostgreSQL, seven packages | 420 passed, 1,497 non-selected tests skipped; one runner leak annotation | `test-postgres-all-verified.log` |
| Git default-branch routes, real isolated PostgreSQL/Redis/MinIO | 4 passed, including fresh clone, CAS races, revocation, replay/outage, and permissions | `test-git-default-branch-live.log` |
| Focused browser cases | 103 distinct cases passed across runs; retry qualification below | `e2e-focused-1.log`, `e2e-retry.log`, `e2e-final-three.log`, `e2e-context-catalog-2.log`, `e2e-empty-team-final.log` |
| PostgreSQL discovery/wrapper checks | Passed | `test-postgres-discovery-final.log`, `test-postgres-wrapper-final.log` |
| Final Rust checks after test-only fixes | Formatting and clippy for buzz-db/buzz-relay passed | `check-rust-final.log` |
| Final browser-source and file-size checks | Passed | `check-e2e-final.log`, `check-file-size-final.log` |
| Migration preservation | All 50 pre-existing files compared byte-for-byte and unchanged | `git diff 960cd5064 -- migrations` contains only 0051 |

The full CI pass preceded the final database fixture/discovery repairs and
browser fixture edits; the affected checks were rerun afterward. No production
code changed after that full CI pass.

The initial 99-case browser run had 70 passes and 29 failures while builds were
running. Rerunning those 29 against a fixed build with one worker yielded 26
passes; after correcting the stale Fleet assertion, the remaining three passed.
Three representative baseline cases and their candidate counterparts also passed.
The four catalog/team integration cases passed across a run and a single-case
retry after updating the empty-team fixture for Fleet's first-agent empty view.
Several retries followed 5-second bridge-startup timeouts. These are passing
individual cases, not a claim of one flake-free 103-case run. An earlier overly
broad browser invocation was cancelled and is not acceptance evidence.

The final PostgreSQL run passed all assertions but nextest marked
`sequence_gap_degrades_and_stops_intake` as leaky (a process/output-handle lifetime
annotation). Its earlier run passed without that annotation. This is not a
failed journal assertion or evidence of a production database leak.
All six journal database tests subsequently passed together without a leak
annotation (`test-journal-final.log`).

The PostgreSQL lane uses a unique desired-state template and one database per
test process, with teardown owned by the runner. The shared relay/desktop database
was not migrated. This is the safe substitute for `just test`, whose infrastructure
bootstrap targets shared local services and whose legacy cargo invocation skips
the ignored PostgreSQL tests.

Reproduction starts with `. ./bin/activate-hermit`. The main gate is `just ci`.
For the isolated database lane, install nextest 0.9.136 and set `PGHOST`, `PGPORT`,
`PGUSER`, `PGPASSWORD`, `PG_BIN_DIR`, and `BUZZ_POSTGRES_ADMIN_URL` to the intended
test cluster, then run `scripts/postgres-test-run.sh`. The nextest profile supplies
database isolation and ignores tests requiring additional external services.
`scripts/test-postgres-test-discovery.sh` and
`scripts/test-postgres-test-wrapper.sh` validate the discovery and isolation
conventions. The gateway readiness test creates and removes its own role/database.

Browser checks use an immutable copy of an E2E-enabled desktop build, a dedicated
localhost preview port, and one Playwright worker. They mock native IPC and do not
establish installed-provider acceptance. Selected smoke files cover onboarding
defaults, lost identity, exact-key profiles, harness management, Codex effort,
and agent creation; focused integration cases cover catalog policy propagation,
unknown-policy fallback, header controls, and empty-team avatar rendering.

## Required installation and deployment follow-up

1. **Conversation-policy transition:** upstream deliberately does not migrate the
   old global `threadScopedAcpSessions` override. This integration likewise does
   not mutate stored user data. Before installing, inspect that override for each
   affected workspace. If enabled, explicitly set existing definitions and
   standalone/orphaned agent records that depended on it to `thread` before agent
   restoration. Linked instances inherit their definition. Missing new policy
   fields otherwise default to `channel`. Do not treat this merge as approval to
   silently change those running agents' conversation scope.
2. **Adapters:** validate the real Pi transition (`buzz-pi-acp`, pinned revision
   `86b201e`, Node 22+) and any Codex adapter update separately. Local ACP fixtures
   do not prove a real installed provider's prompt, skills, resume, or account flow.
3. **Push rollout:** configure the gateway origin consistently. The new bootstrap
   guard refuses a populated pre-launch gateway database rather than upgrading it
   destructively. Any existing deployment needs an explicit state review.
4. **Mobile distribution:** prepare the iOS Declared Age Range capability/signing
   profile and validate native store age signals on supported devices. Flutter
   tests and desktop builds do not establish mobile signing or device behavior.

No app installation, adapter replacement, production database operation, relay
restart, external publication, or deployment is included in this merge.
