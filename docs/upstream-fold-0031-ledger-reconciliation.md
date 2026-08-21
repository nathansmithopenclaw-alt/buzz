# Upstream fold: version-31 migration-ledger reconciliation

This fold preserves upstream's published migration checksums through version
31. Upstream version 31 adds workflow-run error codes. The unchanged Smith
Crafts `community_hosts` migration moves from version 31 to version 32.

The Smith Crafts production database is expected to have the pre-fold
`community_hosts` schema and a successful `_sqlx_migrations` row at version 31
with the community-host checksum. That state must be verified immediately
before deployment; it is not inferred from source history.

## Reconciliation implemented by the relay

`buzz_db::migration::run_migrations` holds the exclusive schema/destruction
session lock for the complete run. Before SQLx validates checksums, the fold
reconciler locks `_sqlx_migrations` and accepts only these version-31 states:

- version 31 is absent;
- version 31 has upstream's canonical workflow-error-code checksum; or
- version 31 has the exact community-host checksum embedded at version 32.

For the third state, the reconciler moves the successful community-host ledger
row from version 31 to 32 and preserves the `community hosts` description. SQLx
then applies upstream version 31. The schema is not reverted or replayed. A
failed or unknown version-31 row, an unknown or failed version-32 row, or
community hosts recorded at both versions fails closed.

The older recognized Smith rows at versions 27 or 29 also converge directly to
version 32. Canonical upstream checksums at versions 27 and 29 remain untouched.

## Production gate and rollout sequence

`BUZZ_AUTO_MIGRATE` remains **off** during normal relay boot. Merging this
source does not authorize a production change.

1. Pin the exact fold commit and artifact. Confirm its embedded migration set
   is versions 1 through 32 and retain a database backup or snapshot identifier.
2. With the existing production binary still serving, capture the preflight:

   ```sql
   SELECT version, description, success, encode(checksum, 'hex') AS checksum
   FROM _sqlx_migrations
   WHERE version IN (27, 29, 30, 31, 32)
   ORDER BY version;

   SELECT to_regclass('community_hosts') AS community_hosts,
          to_regclass('workflow_runs') AS workflow_runs;

   SELECT column_name
   FROM information_schema.columns
   WHERE table_schema = current_schema()
     AND table_name = 'workflow_runs'
     AND column_name = 'error_code';

   SELECT count(*) AS host_rows FROM community_hosts;
   ```

   Expected Smith brownfield shape: canonical upstream rows through version 30;
   successful `community hosts` at version 31 with the exact version-32
   community-host checksum; no row at version 32; and `community_hosts`
   present. The `error_code` column query returns no row. Any difference stops
   the rollout for investigation.
3. Drain normal relay traffic and start exactly one fold relay instance with
   `BUZZ_AUTO_MIGRATE=true`.
4. Require a clean migration exit, then turn `BUZZ_AUTO_MIGRATE` back off
   before starting the normal replica set on the fold artifact.
5. Capture the postflight:

   ```sql
   SELECT version, description, success, encode(checksum, 'hex') AS checksum
   FROM _sqlx_migrations
   WHERE version BETWEEN 27 AND 32
   ORDER BY version;

   SELECT to_regclass('community_hosts') AS community_hosts,
          to_regclass('workflow_runs') AS workflow_runs;

   SELECT count(*) AS host_rows FROM community_hosts;

   SELECT column_name
   FROM information_schema.columns
   WHERE table_schema = current_schema()
     AND table_name = 'workflow_runs'
     AND column_name = 'error_code';
   ```

   Versions 27 through 32 must all be successful and checksum-identical to the
   pinned artifact. Version 31 must describe workflow-run error codes; version
   32 must describe community hosts. The host-row count must be unchanged and
   the column query must return `error_code`.

## Failure and rollback boundary

The ledger move is a small committed transaction that precedes SQLx migration
31. Once it commits, an artifact whose embedded migration set expects version
31 to contain community hosts is no longer a safe rollback target. If the
canonical version-31 migration then fails, keep traffic drained, fix the
blocking condition, and retry the same fold artifact.

Do not hand-edit checksums, delete ledger rows, replay `community_hosts`, or
selectively reverse the ledger move. After the point of no return, rollback is
restoration of the preflight database snapshot together with the old binary.

## Automated evidence required before this plan is used

- a fresh database applies versions 1 through 32;
- an upstream-through-31 database applies community hosts as version 32;
- a Smith database with community hosts at version 31 moves that row to 32,
  applies upstream workflow error codes at 31, and preserves host data;
- legacy Smith rows at versions 27 and 29 converge to version 32; and
- unknown, failed, and duplicate version-31/version-32 shapes fail closed.
