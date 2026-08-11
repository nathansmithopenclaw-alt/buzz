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
if (first.actions.create_issues !== 2 || first.actions.reconcile_pilots !== 1) process.exit(1);
if (!first.pilot_handling.includes("create Tracker issues for every pilot")) process.exit(1);
' "$scratch/plan-one.json" "$scratch/plan-two.json"

plan_hash="$(node -e 'console.log(JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")).plan_sha256)' "$scratch/plan-one.json")"

apply_checkpoint="$scratch/apply-checkpoint.json"
apply_log="$scratch/apply.log"
apply_count="$scratch/apply-count"
TRACKER_MOCK_LOG="$apply_log" TRACKER_MOCK_COUNT="$apply_count" BUZZ_BIN="$fixture_root/mock-buzz.sh" \
  node "$repo_root/scripts/migrate-linear-tracker.ts" apply "${common[@]}" \
  --confirm-plan-sha256 "$plan_hash" --checkpoint "$apply_checkpoint" \
  --report "$scratch/apply.json" > "$scratch/apply-stdout.json"
node -e '
const fs = require("fs");
const report = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const checkpoint = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const mutations = fs.readFileSync(process.argv[3], "utf8").trim().split("\n");
if (!report.passed || report.records.length !== 7) process.exit(1);
if (Object.keys(checkpoint.completed).length !== 7 || mutations.length !== 7) process.exit(1);
' "$scratch/apply.json" "$apply_checkpoint" "$apply_log"
if compgen -G "$apply_checkpoint.tmp-*" > /dev/null; then
  echo "atomic checkpoint write left a temp file" >&2
  exit 1
fi

corrupt_checkpoint="$scratch/corrupt-checkpoint.json"
printf '{not-json\n' > "$corrupt_checkpoint"
if TRACKER_MOCK_LOG="$scratch/corrupt.log" BUZZ_BIN="$fixture_root/mock-buzz.sh" \
  node "$repo_root/scripts/migrate-linear-tracker.ts" apply "${common[@]}" \
  --confirm-plan-sha256 "$plan_hash" --checkpoint "$corrupt_checkpoint" \
  --report "$scratch/corrupt-apply.json" > "$scratch/corrupt-stdout.json" 2> "$scratch/corrupt-stderr.json"; then
  echo "corrupt checkpoint unexpectedly applied" >&2
  exit 1
fi
if [[ -s "$scratch/corrupt.log" ]] || ! grep -q 'cannot parse checkpoint' "$scratch/corrupt-stderr.json"; then
  echo "corrupt checkpoint did not fail before relay calls" >&2
  exit 1
fi

resume_checkpoint="$scratch/resume-checkpoint.json"
resume_log="$scratch/resume.log"
resume_count="$scratch/resume-count"
if TRACKER_MOCK_LOG="$resume_log" TRACKER_MOCK_COUNT="$resume_count" TRACKER_MOCK_FAIL_ON_MUTATION=4 \
  BUZZ_BIN="$fixture_root/mock-buzz.sh" node "$repo_root/scripts/migrate-linear-tracker.ts" apply "${common[@]}" \
  --confirm-plan-sha256 "$plan_hash" --checkpoint "$resume_checkpoint" \
  --report "$scratch/crashed-apply.json" > "$scratch/crashed-stdout.json" 2> "$scratch/crashed-stderr.json"; then
  echo "injected apply failure unexpectedly passed" >&2
  exit 1
fi
TRACKER_MOCK_LOG="$resume_log" TRACKER_MOCK_COUNT="$resume_count" BUZZ_BIN="$fixture_root/mock-buzz.sh" \
  node "$repo_root/scripts/migrate-linear-tracker.ts" apply "${common[@]}" --resume \
  --confirm-plan-sha256 "$plan_hash" --checkpoint "$resume_checkpoint" \
  --report "$scratch/resumed-apply.json" > "$scratch/resumed-stdout.json"
node -e '
const fs = require("fs");
const rows = fs.readFileSync(process.argv[1], "utf8").trim().split("\n").map((line) => line.split("\t").slice(1));
const request = (row) => row[row.indexOf("--request-id") + 1];
const checkpoint = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const report = JSON.parse(fs.readFileSync(process.argv[3], "utf8"));
if (rows.length !== 8 || Object.keys(checkpoint.completed).length !== 7 || !report.passed) process.exit(1);
if (request(rows[3]) !== request(rows[4])) process.exit(1);
for (let i = 0; i < 3; i += 1) if (rows.filter((row) => request(row) === request(rows[i])).length !== 1) process.exit(1);
' "$resume_log" "$resume_checkpoint" "$scratch/resumed-apply.json"
if compgen -G "$resume_checkpoint.tmp-*" > /dev/null; then
  echo "crash-resume checkpoint write left a temp file" >&2
  exit 1
fi

sed 's/"complete_export": true/"complete_export": false/' \
  "$fixture_root/linear-export.json" > "$scratch/incomplete.json"
if node "$repo_root/scripts/migrate-linear-tracker.ts" plan \
  --input "$scratch/incomplete.json" "${common[@]:2}" \
  --report "$scratch/incomplete-report.json" > "$scratch/incomplete-stdout.json" 2>/dev/null; then
  echo "incomplete export unexpectedly passed" >&2
  exit 1
fi

node -e '
const fs = require("fs");
const value = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
value["linear-issue-1"].labels[0] = "priority:P0";
fs.writeFileSync(process.argv[2], JSON.stringify(value));
' "$fixture_root/pilot-map.json" "$scratch/stale-pilot-map.json"
if node "$repo_root/scripts/migrate-linear-tracker.ts" plan \
  "${common[@]:0:11}" "$scratch/stale-pilot-map.json" \
  --report "$scratch/stale-pilot-report.json" > "$scratch/stale-pilot-stdout.json"; then
  echo "stale pilot map unexpectedly passed" >&2
  exit 1
fi
grep -q 'priority diverges from export' "$scratch/stale-pilot-report.json"

node -e '
const fs = require("fs");
const value = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
value.issues[0].comments[0].user.id = "unmapped-comment-author";
fs.writeFileSync(process.argv[2], JSON.stringify(value));
' "$fixture_root/linear-export.json" "$scratch/unmapped-comment.json"
if node "$repo_root/scripts/migrate-linear-tracker.ts" plan \
  --input "$scratch/unmapped-comment.json" "${common[@]:2}" \
  --report "$scratch/unmapped-comment-report.json" > "$scratch/unmapped-comment-stdout.json"; then
  echo "unmapped comment author unexpectedly passed" >&2
  exit 1
fi
grep -q 'unmapped-comment-author' "$scratch/unmapped-comment-report.json"

node -e '
const fs = require("fs");
const value = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const edges = (rows) => ({ edges: (rows ?? []).map((node) => ({ node })) });
for (const issue of value.issues) {
  for (const field of ["labels", "relations", "links", "comments"]) {
    if (issue[field]) issue[field] = edges(issue[field]);
  }
}
value.issues = edges(value.issues);
value.projects = edges(value.projects);
fs.writeFileSync(process.argv[2], JSON.stringify(value));
' "$fixture_root/linear-export.json" "$scratch/edges-export.json"
node "$repo_root/scripts/migrate-linear-tracker.ts" plan \
  --input "$scratch/edges-export.json" "${common[@]:2}" \
  --report "$scratch/edges-plan.json" > "$scratch/edges-stdout.json"
node -e '
const report = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
if (!report.safe_to_apply || report.source.issues !== 2 || report.source.comments !== 1 || report.source.relations !== 1) process.exit(1);
' "$scratch/edges-plan.json"

node -e '
const fs = require("fs");
const value = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
value.issues[0].title = "é".repeat(129);
fs.writeFileSync(process.argv[2], JSON.stringify(value));
' "$fixture_root/linear-export.json" "$scratch/long-title.json"
if node "$repo_root/scripts/migrate-linear-tracker.ts" plan \
  --input "$scratch/long-title.json" "${common[@]:2}" \
  --report "$scratch/long-title-report.json" > "$scratch/long-title-stdout.json"; then
  echo "over-limit UTF-8 title unexpectedly passed" >&2
  exit 1
fi
grep -q '256-byte limit' "$scratch/long-title-report.json"

node -e '
const fs = require("fs");
const value = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
value.issues[0].labels = Array.from({ length: 17 }, (_, index) => ({ name: `label-${index}` }));
fs.writeFileSync(process.argv[2], JSON.stringify(value));
' "$fixture_root/linear-export.json" "$scratch/too-many-labels.json"
if node "$repo_root/scripts/migrate-linear-tracker.ts" plan \
  --input "$scratch/too-many-labels.json" "${common[@]:2}" \
  --report "$scratch/too-many-labels-report.json" > "$scratch/too-many-labels-stdout.json"; then
  echo "over-limit labels unexpectedly passed" >&2
  exit 1
fi
grep -q 'Tracker CLI limit is 16' "$scratch/too-many-labels-report.json"

node -e '
const fs = require("fs");
const value = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
value.comments = [{ id: "orphan-comment", body: "orphan", issue_id: "absent-issue", user: { id: "linear-user-1" } }];
fs.writeFileSync(process.argv[2], JSON.stringify(value));
' "$fixture_root/linear-export.json" "$scratch/orphan-comment.json"
if node "$repo_root/scripts/migrate-linear-tracker.ts" plan \
  --input "$scratch/orphan-comment.json" "${common[@]:2}" \
  --report "$scratch/orphan-comment-report.json" > "$scratch/orphan-comment-stdout.json"; then
  echo "comment for absent issue unexpectedly passed" >&2
  exit 1
fi
grep -q 'references absent issue' "$scratch/orphan-comment-report.json"

BUZZ_BIN="$fixture_root/mock-buzz.sh" node "$repo_root/scripts/migrate-linear-tracker.ts" verify \
  "${common[@]}" --report "$scratch/verify.json" > "$scratch/verify-stdout.json"
node -e '
const fs = require("fs");
const report = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
if (!report.passed || report.pilot_links_verified !== 1 || report.closed_history_verified !== 1) process.exit(1);
for (const key of ["missing_source_ids", "unexpected_source_ids", "unexpected_tracker_issues", "duplicate_source_ids", "field_mismatches", "provenance_mismatches", "comment_mismatches", "relation_mismatches", "link_mismatches", "project_mismatches"]) {
  if (report[key].length !== 0) process.exit(1);
}
' "$scratch/verify.json"

if TRACKER_MOCK_UNEXPECTED=1 BUZZ_BIN="$fixture_root/mock-buzz.sh" \
  node "$repo_root/scripts/migrate-linear-tracker.ts" verify "${common[@]}" \
  --report "$scratch/unexpected-verify.json" > "$scratch/unexpected-verify-stdout.json"; then
  echo "Tracker issue without a Linear ref unexpectedly verified" >&2
  exit 1
fi
node -e '
const report = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
if (report.tracker_issue_count !== 3 || report.unexpected_tracker_issues.length !== 1) process.exit(1);
' "$scratch/unexpected-verify.json"

if TRACKER_MOCK_BAD=1 BUZZ_BIN="$fixture_root/mock-buzz.sh" \
  node "$repo_root/scripts/migrate-linear-tracker.ts" verify "${common[@]}" \
  --report "$scratch/bad-verify.json" > "$scratch/bad-verify-stdout.json"; then
  echo "mismatched Tracker projection unexpectedly verified" >&2
  exit 1
fi

if TRACKER_MOCK_BAD_PROVENANCE=1 BUZZ_BIN="$fixture_root/mock-buzz.sh" \
  node "$repo_root/scripts/migrate-linear-tracker.ts" verify "${common[@]}" \
  --report "$scratch/bad-provenance-verify.json" > "$scratch/bad-provenance-stdout.json"; then
  echo "mismatched import provenance unexpectedly verified" >&2
  exit 1
fi
node -e '
const report = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
const issueFields = new Set(report.provenance_mismatches.map((row) => row.field));
const commentFields = new Set(report.comment_mismatches.map((row) => row.field));
for (const field of ["imported_at", "source_created_at", "archived", "legacy_labels"]) {
  if (!issueFields.has(field)) process.exit(1);
}
if (!commentFields.has("import.original_author")) process.exit(1);
' "$scratch/bad-provenance-verify.json"

echo "migration plan fixtures passed"
