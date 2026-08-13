# Upstream fold: production migration-ledger reconciliation

This fold preserves upstream's published migration checksums at versions 29
and 30 (`community_deletion` and `community_deletion_recovery`) and renumbers
the unchanged Smith Crafts `community_hosts` migration from 29 to 31.

The Smith Crafts production database is expected to have the pre-fold
`community_hosts` schema and a successful `_sqlx_migrations` row at version 29.
That state must be verified immediately before deployment; it is not inferred
from source history.

## Reconciliation implemented by the relay

`buzz_db::migration::run_migrations` holds the exclusive schema/destruction
session lock for the complete run. Before SQLx validates checksums, the fold
reconciler locks `_sqlx_migrations` and accepts only these known states:

- version 27 has the canonical upstream checksum, or the exact legacy
  `community_hosts` checksum pinned from Smith Crafts main `b7471a7e`;
- version 29 has the canonical upstream deletion checksum, or that exact
  legacy `community_hosts` checksum;
- version 31 is absent, or has the canonical `community_hosts` checksum.

If `community_hosts` is recorded at 27 or 29, its successful ledger row is
moved to version 31, its description remains `community hosts`, and its
checksum is advanced to the fold's canonical version-31 checksum. SQLx then
applies any missing canonical migrations in order, including upstream 29 and
30. The runner finally installs the fold's only added version-31 delta: the
`community_hosts` operator-global classification in the deletion taxonomy.
No table or host data is dropped or replayed. Failed rows, unknown checksums,
or duplicate community-host rows fail closed.

## Production gate and rollout sequence

`BUZZ_AUTO_MIGRATE` remains **off** during normal relay boot. Helm owns this
operator sequence; merging this source does not authorize a production change.

1. Pin the exact fold commit and artifact. Confirm its embedded migration set
   is versions 1 through 31 and retain a database backup/snapshot identifier.
2. With the existing production binary still serving, capture the preflight:

   ```sql
   SELECT version, description, success, encode(checksum, 'hex') AS checksum
   FROM _sqlx_migrations
   WHERE version IN (27, 28, 29, 30, 31)
   ORDER BY version;

   SELECT to_regclass('community_hosts') AS community_hosts,
          to_regclass('community_deletion_requests') AS deletion_requests;

   SELECT count(*) AS host_rows FROM community_hosts;
   ```

   Expected Smith brownfield shape: canonical 27 and 28; successful
   `community hosts` at 29 with the exact legacy checksum from `b7471a7e`
   (`9fbe56cf998e83527ac1cc343dfb0ddeda883c32a5656f01412aed448cc27e566875eb017e65f1c2ace41dc2cae36b5d`);
   no rows at 30 or 31; `community_hosts` present; deletion control-plane
   tables absent. Any difference stops the rollout for investigation.
3. Drain normal relay traffic and start exactly one fold relay instance with
   `BUZZ_AUTO_MIGRATE=true`. Migration 29 deliberately uses a five-second
   lock timeout; a timeout is a failed gate, not a reason to bypass the lock.
4. Require a clean migration exit, then turn `BUZZ_AUTO_MIGRATE` back off
   before starting the normal replica set on the fold artifact.
5. Capture the postflight:

   ```sql
   SELECT version, description, success, encode(checksum, 'hex') AS checksum
   FROM _sqlx_migrations
   WHERE version BETWEEN 27 AND 31
   ORDER BY version;

   SELECT to_regclass('community_hosts') AS community_hosts,
          to_regclass('community_deletion_requests') AS deletion_requests,
          to_regclass('community_deletion_approvals') AS deletion_approvals;

   SELECT count(*) AS host_rows FROM community_hosts;

   SELECT deletion_state, count(*)
   FROM communities
   GROUP BY deletion_state
   ORDER BY deletion_state;
   ```

   Versions 27 through 31 must all be successful and checksum-identical to the
   pinned artifact. The host-row count must be unchanged, deletion tables must
   exist, and this schema migration must not create a deletion request or move
   any community out of `active`.

## Failure and rollback boundary

The ledger move is a small committed transaction that precedes SQLx migration
29. Once it commits, the old binary is no longer a safe rollback target: its
embedded migration set does not know version 31. If migration 29 or 30 then
fails, keep traffic drained, fix the blocking condition, and retry the same
fold artifact. Migration 29 is atomic, so its schema is not partially exposed.

Do not hand-edit checksums, delete ledger rows, replay `community_hosts`, or
selectively reverse the ledger move. The emergency rollback after the
point-of-no-return is restoration of the preflight database snapshot together
with the old binary. Before the ledger move, the old artifact and untouched
database remain a valid rollback pair.

## Automated evidence required before this operator plan is used

- fresh database applies all 31 migrations;
- upstream-through-30 database applies `community_hosts` as 31;
- Smith database with `community_hosts` recorded at 29 moves that row to 31,
  applies upstream 29/30, preserves aliases, and ends with versions 1-31;
- legacy Smith database with `community_hosts` recorded at 27 converges to the
  same state;
- unknown, failed, and duplicate ledger shapes fail closed;
- desired-state schema and migration 29 deletion surfaces remain structurally
  equivalent.
