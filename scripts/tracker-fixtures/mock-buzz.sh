#!/usr/bin/env bash
set -euo pipefail

issue_one="7b7f3d7f-2e4e-5dc8-bd56-3282324a3e2b"
issue_two="8eb4bcb7-fd95-5d89-a003-a43365260e59"
export_sha="d6130297b2e2c46c6bc68367229fdff99b317918c8448645ff4d6e62bfb11d83"
issue_one_title="Imported open issue"
issue_one_imported_at=1785672000
issue_one_created_at="2026-08-01T12:00:00Z"
issue_one_legacy_priority="priority:P1"
issue_two_archived=true
comment_author="linear-user-1"
if [[ "${TRACKER_MOCK_BAD:-}" == "1" ]]; then
  issue_one_title="Corrupted title"
fi
if [[ "${TRACKER_MOCK_BAD_PROVENANCE:-}" == "1" ]]; then
  issue_one_imported_at=1
  issue_one_created_at="2026-08-01T12:00:01Z"
  issue_one_legacy_priority="priority:P0"
  issue_two_archived=false
  comment_author="unmapped-comment-author"
fi

arg_value() {
  local wanted="$1"
  shift
  while (( $# )); do
    if [[ "$1" == "$wanted" ]]; then
      shift
      [[ $# -gt 0 ]] || return 1
      printf '%s' "$1"
      return 0
    fi
    shift
  done
  return 1
}

record_mutation() {
  if [[ -n "${TRACKER_MOCK_LOG:-}" ]]; then
    printf 'mutation' >> "$TRACKER_MOCK_LOG"
    printf '\t%s' "$@" >> "$TRACKER_MOCK_LOG"
    printf '\n' >> "$TRACKER_MOCK_LOG"
  fi
  if [[ -n "${TRACKER_MOCK_COUNT:-}" ]]; then
    local count=0
    if [[ -f "$TRACKER_MOCK_COUNT" ]]; then
      read -r count < "$TRACKER_MOCK_COUNT"
    fi
    count=$((count + 1))
    printf '%s\n' "$count" > "$TRACKER_MOCK_COUNT"
    if [[ -n "${TRACKER_MOCK_FAIL_ON_MUTATION:-}" && "$count" == "$TRACKER_MOCK_FAIL_ON_MUTATION" ]]; then
      printf '{"error":"injected mutation failure","index":%s}\n' "$count" >&2
      exit 1
    fi
  fi
}

mutation_receipt() {
  local request_id
  request_id="$(arg_value --request-id "$@")"
  printf '{"accepted":true,"request_id":"%s","event_id":"%064d"}\n' "$request_id" 1
}

if [[ "$1 $2" == "track list" ]]; then
  printf '{"items":[{"issue_id":"%s"},{"issue_id":"%s"}],"next_cursor":null}\n' "$issue_one" "$issue_two"
  exit 0
fi

if [[ "$1 $2 $3" == "track project list" ]]; then
  printf '%s\n' '{"items":[{"project_id":"22222222-2222-4222-8222-222222222222","name":"Dispatch","slug":"dispatch","statuses":["triage","backlog","todo","in_progress","in_review","done","cancelled"],"default_channel_id":"33333333-3333-4333-8333-333333333333","default_verify":"Exact-head verification"}],"next_cursor":null}'
  exit 0
fi

if [[ "$1 $2 $3" == "track project get" ]]; then
  printf '%s\n' '{"error":"project not found"}' >&2
  exit 1
fi

if [[ "$1 $2 $3" == "track project create" ]]; then
  record_mutation "$@"
  mutation_receipt "$@"
  exit 0
fi

if [[ "$1 $2" == "track create" || "$1 $2" == "track comment" || "$1 $2" == "track relate" || "$1 $2" == "track link" ]]; then
  record_mutation "$@"
  mutation_receipt "$@"
  exit 0
fi

if [[ "$1 $2" == "track get" ]]; then
  joined=" $* "
  if [[ "$joined" == *" --include-comments "* ]]; then
    printf '%s\n' '{"error":"--include-comments requires an explicit boolean"}' >&2
    exit 1
  fi
  if [[ "$joined" == *" $issue_one "* ]]; then
    printf '%s\n' "{\"issue\":{\"issue_id\":\"$issue_one\",\"identifier\":\"SC-1\",\"title\":\"$issue_one_title\",\"body\":\"Source body\",\"status\":\"in_progress\",\"priority\":\"P1\",\"assignee_pubkey\":\"1111111111111111111111111111111111111111111111111111111111111111\",\"project\":{\"project_id\":\"22222222-2222-4222-8222-222222222222\",\"slug\":\"dispatch\"},\"labels\":[\"area:tracker\"],\"due\":\"2026-08-20\",\"home_channel_id\":\"33333333-3333-4333-8333-333333333333\",\"external_refs\":[{\"type\":\"linear\",\"ref\":\"linear-issue-1\"},{\"type\":\"pull_request\",\"ref\":\"https://example.test/pr/1\"},{\"type\":\"git_issue\",\"ref\":\"4444444444444444444444444444444444444444444444444444444444444444\"}],\"relations\":[{\"type\":\"blocks\",\"target_issue_id\":\"$issue_two\",\"target_root_event\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}],\"activity\":[{\"type\":\"created\",\"import_provenance\":{\"system\":\"linear\",\"external_id\":\"linear-issue-1\",\"source_url\":null,\"imported_at\":$issue_one_imported_at,\"source_created_at\":\"$issue_one_created_at\",\"source_updated_at\":\"2026-08-02T12:00:00Z\",\"estimate_points\":3,\"export_sha256\":\"$export_sha\",\"importer_version\":\"buzz-linear-tracker-v1\",\"original_author\":\"linear-user-1\",\"legacy_labels\":[\"$issue_one_legacy_priority\",\"due:2026-08-20\",\"project:dispatch\",\"linear:linear-issue-1\"]}}],\"comments\":[{\"body\":\"Imported comment\",\"import\":{\"system\":\"linear\",\"external_id\":\"linear-comment-1\",\"source_url\":null,\"imported_at\":1785672000,\"source_created_at\":\"2026-08-01T13:00:00Z\",\"export_sha256\":\"$export_sha\",\"importer_version\":\"buzz-linear-tracker-v1\",\"original_author\":\"$comment_author\"}}]}}"
  else
    printf '%s\n' "{\"issue\":{\"issue_id\":\"$issue_two\",\"identifier\":\"SC-2\",\"title\":\"Imported closed issue\",\"body\":\"Closed source body\",\"status\":\"done\",\"priority\":null,\"assignee_pubkey\":null,\"project\":{\"project_id\":\"22222222-2222-4222-8222-222222222222\",\"slug\":\"dispatch\"},\"labels\":[],\"due\":null,\"home_channel_id\":\"33333333-3333-4333-8333-333333333333\",\"external_refs\":[{\"type\":\"linear\",\"ref\":\"linear-issue-2\"}],\"relations\":[],\"activity\":[{\"type\":\"created\",\"import_provenance\":{\"system\":\"linear\",\"external_id\":\"linear-issue-2\",\"source_url\":null,\"imported_at\":1785672000,\"source_created_at\":\"2026-07-01T12:00:00Z\",\"source_updated_at\":\"2026-07-03T12:00:00Z\",\"source_completed_at\":\"2026-07-03T12:00:00Z\",\"archived\":$issue_two_archived,\"export_sha256\":\"$export_sha\",\"importer_version\":\"buzz-linear-tracker-v1\",\"original_author\":\"linear-user-1\"}}],\"comments\":[]}}"
  fi
  exit 0
fi

printf '{"error":"unexpected mock invocation","args":"%s"}\n' "$*" >&2
exit 1
