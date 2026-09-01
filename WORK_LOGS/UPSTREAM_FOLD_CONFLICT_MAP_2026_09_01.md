---
title: "Upstream Fold Conflict Map 2026-09-01"
tags: [upstream-fold, merge-conflicts, buzz-acp, migrations]
status: active
created: 2026-09-01
---

# Reproduction

- Canonical parent: `97f28497c92f3878b7dd172d7dafcecfac50e62c`
- Upstream parent: `5aed49b505a7e27f3b0e34dafa53d6c4e8cdcd64`
- Merge base: `c3132c3ee982d194cd0198ad07b57ec8bd726e4e`
- Command: `git merge-tree --write-tree --messages --name-only 97f28497c92f3878b7dd172d7dafcecfac50e62c 5aed49b505a7e27f3b0e34dafa53d6c4e8cdcd64`
- Synthetic merge tree: `295d3c3d08c7bd4c5478cb0b9014462911981650`
- Result: exit 1, 38 conflicted paths: 36 content conflicts and 2 modify/delete conflicts.

# Conflicted paths

1. `Cargo.lock`
2. `Justfile`
3. `crates/buzz-acp/src/lib.rs`
4. `crates/buzz-acp/src/pool.rs`
5. `crates/buzz-acp/src/queue.rs`
6. `crates/buzz-agent/tests/fake_llm.rs`
7. `crates/buzz-db/src/runtime/migration.rs`
8. `crates/buzz-db/src/runtime/mod.rs`
9. `crates/buzz-relay/src/api/bridge.rs`
10. `crates/buzz-relay/src/api/gifs.rs`
11. `crates/buzz-relay/src/api/invites.rs`
12. `crates/buzz-relay/src/api/workflows.rs`
13. `crates/buzz-relay/src/connection.rs`
14. `desktop/playwright.config.ts`
15. `desktop/src-tauri/src/app_state.rs`
16. `desktop/src-tauri/src/commands/agent_models_tests.rs`
17. `desktop/src-tauri/src/commands/agents_profile.rs`
18. `desktop/src-tauri/src/commands/agents_tests.rs`
19. `desktop/src-tauri/src/commands/personas/snapshot/tests.rs`
20. `desktop/src-tauri/src/commands/personas/update.rs`
21. `desktop/src-tauri/src/commands/workspace.rs`
22. `desktop/src-tauri/src/managed_agents/discovery/tests.rs`
23. `desktop/src-tauri/src/managed_agents/reserved_env_keys.rs`
24. `desktop/src-tauri/src/managed_agents/runtime.rs`
25. `desktop/src-tauri/src/managed_agents/runtime/tests.rs`
26. `desktop/src-tauri/src/relay.rs`
27. `desktop/src-tauri/src/relay/tests.rs`
28. `desktop/src/features/agents/ui/AgentIdentityCard.tsx`
29. `desktop/src/features/agents/ui/UnifiedAgentsSection.tsx` (modify/delete)
30. `desktop/src/features/channels/ui/AddMemberSearchResultRow.tsx` (modify/delete)
31. `desktop/src/features/communities/useCommunityInit.ts`
32. `desktop/src/features/settings/ui/ExperimentalFeaturesCard.tsx`
33. `desktop/src/shared/api/tauri.ts`
34. `desktop/src/shared/api/types.ts`
35. `desktop/tests/e2e/agents.spec.ts`
36. `schema/schema.sql`
37. `scripts/check-file-sizes-core.test.mjs`
38. `scripts/run-tests.sh`

# Resolution record

All 38 paths were resolved in the lane worktree. The two modify/delete paths
(`UnifiedAgentsSection.tsx` and `AddMemberSearchResultRow.tsx`) remain deleted;
their upstream behavior already lives in the replacement component structure.
Non-ACP conflicts were resolved by retaining canonical product/runtime behavior
and adding upstream capabilities around it. The material hand-merged seams were:

- relay rejection correlation: upstream's shared EVENT/REQ rejection parser was
  retained while canonical duplicate-rejection behavior remains authoritative;
- relay bridge/auth: upstream public-scheme support was combined with canonical
  signed-timestamp validation through `SubmitEventAuth`;
- managed-agent descriptions: upstream `description`/kind-0 `about` support was
  added without dropping canonical snapshot metadata, avatar, experiment guard,
  persona identity, or linked-agent behavior;
- desktop activity: the canonical embedded activity read model remains, while
  the existing “View activity” control again opens the full agent-session panel;
- runtime logging: upstream's `child_env`-aware filtering replaced the older
  duplicate filter without weakening canonical managed-child behavior.

## buzz-acp conflict hunks

`git merge-file -p --diff3` against the two parents and merge base reproduced
23 ACP hunks: 7 in `lib.rs`, 4 in `pool.rs`, and 12 in `queue.rs`.

### `crates/buzz-acp/src/lib.rs` (7)

1. Module declarations: retained canonical `runtime_identity` and added upstream
   `scope`.
2. Listener authorization: retained canonical exact-mention, sibling, workflow,
   and answered-state gates; integrated upstream's normalized authorized ingress
   only after those gates, so thread scoping cannot change eligibility.
3. Queue admission: derive upstream `SessionScope` once at admission and enqueue
   by scope, while retaining canonical author capture, dedup outcome, and visible
   queue-loss reporting.
4. Seen/steer handling: retained canonical “seen only when accepted” behavior and
   loss notices; dispatches upstream steer/flush by the admitted exact scope.
5. Shutdown/control helpers: retained canonical visible shutdown interruption
   notices and added upstream exact-scope control signaling.
6. Flush scheduling: retained canonical skip/parking semantics and added upstream
   held scoped batches, preventing a busy scope owner from spawning a duplicate
   provider session.
7. Tests: retained canonical exact-mention/sibling authorization fixtures and
   added upstream forged-workflow attribution coverage.

### `crates/buzz-acp/src/pool.rs` (4)

1. Pool state: retained canonical exclusive-session and one-time park notices;
   added upstream `SessionScope -> worker` ownership.
2. Initialization/helpers: initialized both policy states and retained exact-scope
   owner liveness checks.
3. Claiming: scope affinity is the first pass; canonical exclusive policy still
   prevents an idle worker holding another scope from being reassigned, and the
   caller receives `None` for visible parking rather than forking a session.
4. Turn context/tests: retained canonical publication-watch setup and fixtures;
   used upstream scope-derived observer channel and added conversation-scope test
   helpers.

### `crates/buzz-acp/src/queue.rs` (12)

1. State maps: converted deadlines, batch sizes, retries, and recoverable
   in-flight copies to `SessionScope` keys while retaining canonical loss records.
2. Push/cap: enqueue and cap per exact scope; every evicted event still creates a
   canonical visible overflow loss.
3. Deadline expiry in flush: expire by scope and emit canonical visible in-flight
   loss with the scope's channel and exact event count.
4. Candidate selection: apply canonical skip/parking filtering to scoped queues,
   then reject in-flight or backoff scopes.
5. Cancelled candidate selection: choose a non-skipped, non-in-flight scope rather
   than a flat channel.
6. Cancelled re-dispatch: restore the exact scope and its original timestamps,
   deadline, batch count, and recoverable payload.
7. Batch drain: drain only one exact thread scope, preserving canonical
   oldest-first order and batch cap; sibling thread roots remain queued.
8. Completion: clear all in-flight/recoverable state for the exact scope and
   preserve canonical retry cleanup.
9. Deadline expiry in readiness: same scope-keyed visible-loss behavior as the
   flush path.
10. Work visibility (queued/cancelled): apply canonical visibility/skip policy to
    each scope and exclude scopes already in flight.
11. Work visibility (withheld steer): withheld native-steer work is visible only
    for a permitted, non-in-flight exact scope.
12. Helpers/tests: retained canonical `take_losses` and overflow-loss regression;
    generalized count helpers to scope-or-channel and added upstream timestamped
    requeue coverage.

The merged ACP suite passed with all ambient `BUZZ_ACP_*` variables removed:
980 library tests plus 9 pool lifecycle tests passed, with 2 ignored.

# Migration reconciliation and isolated proof

| Upstream path | Folded path | Body check |
|---|---|---|
| `0041_nip_fi_identity_foundation.sql` | `0045_nip_fi_identity_foundation.sql` | byte-identical (`cmp` exit 0) |
| `0042_nip_fi_authorization_foundation.sql` | `0046_nip_fi_authorization_foundation.sql` | byte-identical (`cmp` exit 0) |
| `0043_push_gateway_dogfood_profile.sql` | `0047_push_gateway_dogfood_profile.sql` | byte-identical (`cmp` exit 0) |

`migration.rs` now reconciles 47 migrations and tests the brownfield boundary.
Because byte-identical upstream 0046 predates canonical `community_hosts`, its
function replacement temporarily omits that table. The existing post-SQLx
reconciliation now restores the desired-state union of `community_hosts` plus
all NIP-FI ledger tables; the ledger-44 contract asserts every member remains
write-fence excluded.
The dedicated `buzz_bolt_fold` database on isolated Postgres port 5471 was
prepared through ledger 44. A preserve-schema boot via
`scripts/start-isolated-test-relay.sh` started the folded relay on ports
3331/8389/9503 and produced exactly ledger rows 45, 46, and 47. The relay was
then stopped and all three proof ports were checked for zero listeners. The
module-qualified direct contract test also passed 1/1:

```text
BUZZ_TEST_DATABASE_URL=postgres://buzz:***@localhost:5471/buzz_bolt_fold \
  cargo test -p buzz-db \
  runtime::migration::postgres_tests::ledger_44_upgrade_applies_exactly_0045_through_0047 \
  -- --ignored --exact --nocapture
```
