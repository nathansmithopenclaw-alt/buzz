#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
fixture_root="$repo_root/scripts/tracker-fixtures"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

common=(
  --input "$fixture_root/linear-export.json"
  --workspace 55555555-5555-4555-8555-555555555555
  --state-map "$fixture_root/state-map.json"
  --user-map "$fixture_root/user-map.json"
  --project-map "$fixture_root/project-map.json"
  --pilot-map "$fixture_root/pilot-map.json"
)

node "$repo_root/scripts/migrate-linear-tracker.ts" plan "${common[@]}" \
  --report "$scratch/plan-one.json" > "$scratch/stdout-one.json"
node "$repo_root/scripts/migrate-linear-tracker.ts" plan "${common[@]}" \
  --report "$scratch/plan-two.json" > "$scratch/stdout-two.json"

node -e '
const fs = require("fs");
const first = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const second = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
if (!first.safe_to_apply || first.plan_sha256 !== second.plan_sha256) process.exit(1);
if (first.source.issues !== 2 || first.source.comments !== 1 || first.source.relations !== 1 || first.source.links !== 1) process.exit(1);
if (first.lifecycle.terminal !== 1 || first.lifecycle.non_terminal !== 1 || first.lifecycle.archived !== 1) process.exit(1);
' "$scratch/plan-one.json" "$scratch/plan-two.json"

sed 's/"complete_export": true/"complete_export": false/' \
  "$fixture_root/linear-export.json" > "$scratch/incomplete.json"
if node "$repo_root/scripts/migrate-linear-tracker.ts" plan \
  "${common[@]/$fixture_root\/linear-export.json/$scratch\/incomplete.json}" \
  --report "$scratch/incomplete-report.json" > "$scratch/incomplete-stdout.json" 2>/dev/null; then
  echo "incomplete export unexpectedly passed" >&2
  exit 1
fi

BUZZ_BIN="$fixture_root/mock-buzz.sh" node "$repo_root/scripts/migrate-linear-tracker.ts" verify \
  "${common[@]}" --report "$scratch/verify.json" > "$scratch/verify-stdout.json"
node -e '
const fs = require("fs");
const report = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
if (!report.passed || report.pilot_links_verified !== 1 || report.closed_history_verified !== 1) process.exit(1);
for (const key of ["missing_source_ids", "unexpected_source_ids", "duplicate_source_ids", "field_mismatches", "provenance_mismatches", "comment_mismatches", "relation_mismatches", "link_mismatches", "project_mismatches"]) {
  if (report[key].length !== 0) process.exit(1);
}
' "$scratch/verify.json"

if TRACKER_MOCK_BAD=1 BUZZ_BIN="$fixture_root/mock-buzz.sh" \
  node "$repo_root/scripts/migrate-linear-tracker.ts" verify "${common[@]}" \
  --report "$scratch/bad-verify.json" > "$scratch/bad-verify-stdout.json"; then
  echo "mismatched Tracker projection unexpectedly verified" >&2
  exit 1
fi

echo "migration plan fixtures passed"
