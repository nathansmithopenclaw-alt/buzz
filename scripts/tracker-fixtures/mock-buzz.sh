#!/usr/bin/env bash
set -euo pipefail

issue_one="7b7f3d7f-2e4e-5dc8-bd56-3282324a3e2b"
issue_two="8eb4bcb7-fd95-5d89-a003-a43365260e59"
export_sha="d6130297b2e2c46c6bc68367229fdff99b317918c8448645ff4d6e62bfb11d83"
issue_one_title="Imported open issue"
if [[ "${TRACKER_MOCK_BAD:-}" == "1" ]]; then
  issue_one_title="Corrupted title"
fi

if [[ "$1 $2" == "track list" ]]; then
  printf '{"items":[{"issue_id":"%s"},{"issue_id":"%s"}],"next_cursor":null}\n' "$issue_one" "$issue_two"
  exit 0
fi

if [[ "$1 $2 $3" == "track project list" ]]; then
  printf '%s\n' '{"items":[{"project_id":"22222222-2222-4222-8222-222222222222","name":"Dispatch","slug":"dispatch","statuses":["triage","backlog","todo","in_progress","in_review","done","cancelled"],"default_channel_id":"33333333-3333-4333-8333-333333333333","default_verify":"Exact-head verification"}],"next_cursor":null}'
  exit 0
fi

if [[ "$1 $2" == "track get" ]]; then
  joined=" $* "
  if [[ "$joined" == *" $issue_one "* ]]; then
    printf '%s\n' "{\"issue\":{\"issue_id\":\"$issue_one\",\"identifier\":\"SC-1\",\"title\":\"$issue_one_title\",\"body\":\"Source body\",\"status\":\"in_progress\",\"priority\":\"P1\",\"assignee_pubkey\":\"1111111111111111111111111111111111111111111111111111111111111111\",\"project\":{\"project_id\":\"22222222-2222-4222-8222-222222222222\",\"slug\":\"dispatch\"},\"labels\":[\"area:tracker\"],\"due\":\"2026-08-20\",\"home_channel_id\":\"33333333-3333-4333-8333-333333333333\",\"external_refs\":[{\"type\":\"linear\",\"ref\":\"linear-issue-1\"},{\"type\":\"pull_request\",\"ref\":\"https://example.test/pr/1\"},{\"type\":\"git_issue\",\"ref\":\"4444444444444444444444444444444444444444444444444444444444444444\"}],\"relations\":[{\"type\":\"blocks\",\"target_issue_id\":\"$issue_two\",\"target_root_event\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}],\"activity\":[{\"type\":\"created\",\"import_provenance\":{\"system\":\"linear\",\"external_id\":\"linear-issue-1\",\"export_sha256\":\"$export_sha\"}}],\"comments\":[{\"body\":\"Imported comment\",\"import\":{\"system\":\"linear\",\"external_id\":\"linear-comment-1\"}}]}}"
  else
    printf '%s\n' "{\"issue\":{\"issue_id\":\"$issue_two\",\"identifier\":\"SC-2\",\"title\":\"Imported closed issue\",\"body\":\"Closed source body\",\"status\":\"done\",\"priority\":null,\"assignee_pubkey\":null,\"project\":{\"project_id\":\"22222222-2222-4222-8222-222222222222\",\"slug\":\"dispatch\"},\"labels\":[],\"due\":null,\"home_channel_id\":\"33333333-3333-4333-8333-333333333333\",\"external_refs\":[{\"type\":\"linear\",\"ref\":\"linear-issue-2\"}],\"relations\":[],\"activity\":[{\"type\":\"created\",\"import_provenance\":{\"system\":\"linear\",\"external_id\":\"linear-issue-2\",\"export_sha256\":\"$export_sha\"}}],\"comments\":[]}}"
  fi
  exit 0
fi

printf '{"error":"unexpected mock invocation","args":"%s"}\n' "$*" >&2
exit 1
