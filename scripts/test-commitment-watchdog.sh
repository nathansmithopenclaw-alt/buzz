#!/bin/bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
WATCHDOG="$SCRIPT_DIR/commitment-watchdog.sh"
ROOT=$(mktemp -d)
trap 'rm -rf "$ROOT"' EXIT

SIGNER=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
DELEGATOR=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
OWNER=cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
FALLBACK=dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd
REPLY_TO=eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee

mkdir -p "$ROOT/watch/healthy.lock" "$ROOT/watch/stale.lock" "$ROOT/state" "$ROOT/bin"
touch "$ROOT/healthy.heartbeat" "$ROOT/stale.heartbeat"

cat > "$ROOT/bin/buzz" <<'SH'
#!/bin/bash
set -eu
case "${1:-} ${2:-}" in
  "users get")
    pubkey=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
    [ ! -f "$MOCK_BUZZ_STATE/bad-signer" ] || pubkey=ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
    printf '[{"pubkey":"%s"}]\n' "$pubkey"
    ;;
  "messages search")
    [ ! -f "$MOCK_BUZZ_STATE/fail-search" ] || exit 2
    if [ -f "$MOCK_BUZZ_STATE/relay-event.json" ]; then
      cat "$MOCK_BUZZ_STATE/relay-event.json"
    else
      printf '[]\n'
    fi
    ;;
  "messages send")
    printf '%s\n' "$@" > "$MOCK_BUZZ_STATE/last-send-args"
    content=$(cat)
    printf '%s\n' "$content" > "$MOCK_BUZZ_STATE/last-message.md"
    count=$(cat "$MOCK_BUZZ_STATE/send-count" 2>/dev/null || printf '0')
    count=$((count + 1))
    printf '%s\n' "$count" > "$MOCK_BUZZ_STATE/send-count"
    jq -n --arg content "$content" '[{pubkey:"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",content:$content,tags:[["h","00000000-0000-0000-0000-000000000000"]]}]' > "$MOCK_BUZZ_STATE/relay-event.json"
    printf '{"accepted":true,"event_id":"9999999999999999999999999999999999999999999999999999999999999999"}\n'
    ;;
  *)
    exit 99
    ;;
esac
SH
chmod +x "$ROOT/bin/buzz"

cat > "$ROOT/bin/security" <<'SH'
#!/bin/bash
printf 'test-signing-key\n'
SH
chmod +x "$ROOT/bin/security"
export MOCK_BUZZ_STATE="$ROOT/state/mock-buzz"
mkdir -p "$MOCK_BUZZ_STATE"
export PATH="$ROOT/bin:$PATH"

cat > "$ROOT/config.json" <<JSON
{
  "version": 1,
  "watch_root": "$ROOT/watch",
  "state_root": "$ROOT/state",
  "log_path": "$ROOT/watchdog.log",
  "relay_url": "wss://relay.invalid",
  "buzz_cli": "$ROOT/bin/buzz",
  "keychain_service": "unused-in-tests",
  "expected_signer_pubkey": "$SIGNER",
  "marker_name": ".buzz-commitment.json",
  "claim_ttl_seconds": 30
}
JSON

write_marker() {
  local path="$1" id="$2" heartbeat="$3" epoch="$4"
  cat > "$path" <<JSON
{
  "version": 2,
  "commitment_id": "$id",
  "epoch": "$epoch",
  "description": "$id proof commitment",
  "channel_id": "00000000-0000-0000-0000-000000000000",
  "reply_to": "$REPLY_TO",
  "owner_mention": "$OWNER",
  "delegator_display": "Test Orchestrator",
  "delegator_pubkey": "$DELEGATOR",
  "fallback_mentions": [],
  "stale_after_seconds": 60,
  "heartbeat_paths": ["$heartbeat"]
}
JSON
}

sha256_text() {
  if command -v shasum >/dev/null 2>&1; then
    printf '%s' "$1" | shasum -a 256 | awk '{print $1}'
  else
    printf '%s' "$1" | sha256sum | awk '{print $1}'
  fi
}

action_key() {
  sha256_text "$1|$2|recovery-$3"
}

recovery_state() {
  printf '%s/state/recoveries/%s.json\n' "$ROOT" "$(sha256_text "$1")"
}

make_stale() {
  touch -t 202001010000 "$@"
}

set_mtime_age() {
  local age="$1" path="$2" target stamp
  target=$(( $(date +%s) - age ))
  if stamp=$(date -r "$target" '+%Y%m%d%H%M.%S' 2>/dev/null); then
    :
  else
    stamp=$(date -d "@$target" '+%Y%m%d%H%M.%S')
  fi
  touch -t "$stamp" "$path"
}

set_recovery_notice_age() {
  local id="$1" age="$2" state tmp
  state=$(recovery_state "$id")
  tmp="$state.tmp"
  jq --argjson notified "$(( $(date +%s) - age ))" '.notified_at_epoch = $notified' "$state" > "$tmp"
  mv "$tmp" "$state"
}

assert_arg_pair() {
  local flag="$1" value="$2"
  awk -v flag="$flag" -v value="$value" 'previous == flag && $0 == value { found=1 } { previous=$0 } END { exit(found ? 0 : 1) }' \
    "$MOCK_BUZZ_STATE/last-send-args"
}

assert_no_arg_pair() {
  local flag="$1" value="$2"
  if assert_arg_pair "$flag" "$value"; then
    printf 'unexpected argument pair: %s %s\n' "$flag" "$value" >&2
    exit 1
  fi
}

disarm_all() {
  rm -f "$ROOT/watch/healthy.lock/.buzz-commitment.json" "$ROOT/watch/stale.lock/.buzz-commitment.json"
  "$WATCHDOG" check --config "$ROOT/config.json"
}

write_marker "$ROOT/watch/healthy.lock/.buzz-commitment.json" healthy "$ROOT/healthy.heartbeat" epoch-1
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" stale-dry "$ROOT/stale.heartbeat" epoch-1
"$WATCHDOG" validate --config "$ROOT/config.json" | grep -q '2 commitment marker(s) valid'

# Healthy work remains quiet even when the marker itself is old: a declared
# heartbeat, not elapsed wall time alone, is the liveness contract.
make_stale "$ROOT/watch/healthy.lock/.buzz-commitment.json"
touch "$ROOT/healthy.heartbeat"
make_stale "$ROOT/watch/stale.lock/.buzz-commitment.json" "$ROOT/stale.heartbeat"
COMMITMENT_WATCHDOG_DRY_RUN=1 "$WATCHDOG" check --config "$ROOT/config.json"
outbox_count=$(find "$ROOT/state/outbox" -type f | wc -l | tr -d ' ')
[ "$outbox_count" = 1 ]
grep -q 'stale-dry proof commitment' "$ROOT/state/outbox"/*.md
! grep -q 'healthy proof commitment' "$ROOT/state/outbox"/*.md
grep -q 'Automatic resume requested from the owning agent' "$ROOT/state/outbox"/*.md
grep -Eq 'no declared heartbeat for [0-9]+y [0-9]+d' "$ROOT/state/outbox"/*.md

# Rechecking the same recovery epoch is at-most-once.
COMMITMENT_WATCHDOG_DRY_RUN=1 "$WATCHDOG" check --config "$ROOT/config.json"
outbox_count=$(find "$ROOT/state/outbox" -type f | wc -l | tr -d ' ')
[ "$outbox_count" = 1 ]
disarm_all

# Dry-run state is evidence only and cannot poison a later production owner
# notice for the same marker epoch.
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" dry-then-live "$ROOT/stale.heartbeat" epoch-1
make_stale "$ROOT/watch/stale.lock/.buzz-commitment.json" "$ROOT/stale.heartbeat"
COMMITMENT_WATCHDOG_DRY_RUN=1 "$WATCHDOG" check --config "$ROOT/config.json"
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 1 ]
assert_arg_pair --reply-to "$REPLY_TO"
assert_arg_pair --mention "$OWNER"
assert_no_arg_pair --mention "$DELEGATOR"
grep -q 'Automatic resume requested from the owning agent' "$MOCK_BUZZ_STATE/last-message.md"

# If the owner never picks up, a second full stale threshold escalates to the
# delegator. The owner is not pinged again, and the escalation is deduplicated.
set_recovery_notice_age dry-then-live 61
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 2 ]
assert_arg_pair --mention "$DELEGATOR"
assert_no_arg_pair --mention "$OWNER"
grep -q '@Test Orchestrator COMMITMENT STALLED AGAIN' "$MOCK_BUZZ_STATE/last-message.md"
grep -q 'Automatic owner resume was already attempted once and failed' "$MOCK_BUZZ_STATE/last-message.md"
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 2 ]
disarm_all

# Fallback recipients remain second-epoch-only. The first notice mentions only
# the owner; escalation mentions the delegator and every configured fallback.
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" fallback-routing "$ROOT/stale.heartbeat" epoch-1
jq --arg fallback "$FALLBACK" '.fallback_mentions = [$fallback]' "$ROOT/watch/stale.lock/.buzz-commitment.json" > "$ROOT/marker.tmp"
mv "$ROOT/marker.tmp" "$ROOT/watch/stale.lock/.buzz-commitment.json"
make_stale "$ROOT/watch/stale.lock/.buzz-commitment.json" "$ROOT/stale.heartbeat"
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 3 ]
assert_arg_pair --mention "$OWNER"
assert_no_arg_pair --mention "$DELEGATOR"
assert_no_arg_pair --mention "$FALLBACK"
set_recovery_notice_age fallback-routing 61
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 4 ]
assert_arg_pair --mention "$DELEGATOR"
assert_arg_pair --mention "$FALLBACK"
assert_no_arg_pair --mention "$OWNER"
disarm_all

# A watchdog-triggered wakeup that only heartbeats does not reset recovery.
# Once that heartbeat becomes stale, the next notice escalates to the delegator.
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" heartbeat-then-stale "$ROOT/stale.heartbeat" epoch-1
make_stale "$ROOT/watch/stale.lock/.buzz-commitment.json" "$ROOT/stale.heartbeat"
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 5 ]
touch "$ROOT/stale.heartbeat"
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 5 ]
jq -e '.phase == "owner_notified"' "$(recovery_state heartbeat-then-stale)" >/dev/null
set_recovery_notice_age heartbeat-then-stale 120
set_mtime_age 61 "$ROOT/stale.heartbeat"
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 6 ]
assert_arg_pair --mention "$DELEGATOR"
assert_no_arg_pair --mention "$OWNER"
disarm_all

# An explicit owner re-arm is a new marker epoch plus a fresh heartbeat. It
# resets recovery to phase 0, so a later stall gets one new owner notice.
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" explicit-rearm "$ROOT/stale.heartbeat" epoch-1
make_stale "$ROOT/watch/stale.lock/.buzz-commitment.json" "$ROOT/stale.heartbeat"
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 7 ]
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" explicit-rearm "$ROOT/stale.heartbeat" epoch-2
touch "$ROOT/stale.heartbeat"
"$WATCHDOG" check --config "$ROOT/config.json"
[ ! -f "$(recovery_state explicit-rearm)" ]
make_stale "$ROOT/watch/stale.lock/.buzz-commitment.json" "$ROOT/stale.heartbeat"
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 8 ]
assert_arg_pair --mention "$OWNER"
assert_no_arg_pair --mention "$DELEGATOR"
disarm_all

# Removing the marker normally disarms the recovery episode. Reopening the
# logical commitment begins again with an owner notice.
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" disarm-reset "$ROOT/stale.heartbeat" epoch-1
make_stale "$ROOT/watch/stale.lock/.buzz-commitment.json" "$ROOT/stale.heartbeat"
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 9 ]
rm -f "$ROOT/watch/stale.lock/.buzz-commitment.json"
"$WATCHDOG" check --config "$ROOT/config.json"
[ ! -f "$(recovery_state disarm-reset)" ]
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" disarm-reset "$ROOT/stale.heartbeat" epoch-2
make_stale "$ROOT/watch/stale.lock/.buzz-commitment.json" "$ROOT/stale.heartbeat"
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 10 ]
assert_arg_pair --mention "$OWNER"
disarm_all

# The configured signing-key boundary fails closed if the key resolves to any
# identity other than the explicitly expected, owner-attested signer.
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" wrong-signer "$ROOT/stale.heartbeat" epoch-1
make_stale "$ROOT/watch/stale.lock/.buzz-commitment.json" "$ROOT/stale.heartbeat"
touch "$MOCK_BUZZ_STATE/bad-signer"
if "$WATCHDOG" check --config "$ROOT/config.json"; then
  echo "unexpected signer was allowed to publish" >&2
  exit 1
fi
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 10 ]
rm -f "$MOCK_BUZZ_STATE/bad-signer"
disarm_all

# Simulate a crash after relay acceptance but before local state persistence.
# The relay token recovers the first recovery epoch without a duplicate send.
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" crash-recovery "$ROOT/stale.heartbeat" epoch-1
make_stale "$ROOT/watch/stale.lock/.buzz-commitment.json" "$ROOT/stale.heartbeat"
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 11 ]
owner_key=$(action_key crash-recovery epoch-1 owner)
rm -f "$ROOT/state/alerts/$owner_key.json" "$(recovery_state crash-recovery)"
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 11 ]
jq -e '.status == "observed_on_relay" and .phase == "owner"' "$ROOT/state/alerts/$owner_key.json" >/dev/null
jq -e '.phase == "owner_notified"' "$(recovery_state crash-recovery)" >/dev/null
disarm_all

# A failed dedupe query is ambiguous and fails closed. Once lookup health
# returns, the pending owner notice is delivered normally.
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" failed-search "$ROOT/stale.heartbeat" epoch-1
make_stale "$ROOT/watch/stale.lock/.buzz-commitment.json" "$ROOT/stale.heartbeat"
touch "$MOCK_BUZZ_STATE/fail-search"
if "$WATCHDOG" check --config "$ROOT/config.json"; then
  echo "ambiguous relay lookup unexpectedly allowed the check" >&2
  exit 1
fi
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 11 ]
rm -f "$MOCK_BUZZ_STATE/fail-search"
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 12 ]
assert_arg_pair --mention "$OWNER"
disarm_all

# A signer can never mention itself as owner, delegator, or fallback: those
# self-authored events reach the relay but cannot wake the signer identity.
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" self-owner "$ROOT/stale.heartbeat" epoch-1
jq --arg signer "$SIGNER" '.owner_mention = $signer' "$ROOT/watch/stale.lock/.buzz-commitment.json" > "$ROOT/marker.tmp"
mv "$ROOT/marker.tmp" "$ROOT/watch/stale.lock/.buzz-commitment.json"
if "$WATCHDOG" validate --config "$ROOT/config.json" >/dev/null 2>&1; then
  echo "self-targeting owner unexpectedly passed validation" >&2
  exit 1
fi
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" self-delegator "$ROOT/stale.heartbeat" epoch-1
jq --arg signer "$SIGNER" '.delegator_pubkey = $signer' "$ROOT/watch/stale.lock/.buzz-commitment.json" > "$ROOT/marker.tmp"
mv "$ROOT/marker.tmp" "$ROOT/watch/stale.lock/.buzz-commitment.json"
if "$WATCHDOG" validate --config "$ROOT/config.json" >/dev/null 2>&1; then
  echo "self-targeting delegator unexpectedly passed validation" >&2
  exit 1
fi
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" self-fallback "$ROOT/stale.heartbeat" epoch-1
jq --arg signer "$SIGNER" '.fallback_mentions = [$signer]' "$ROOT/watch/stale.lock/.buzz-commitment.json" > "$ROOT/marker.tmp"
mv "$ROOT/marker.tmp" "$ROOT/watch/stale.lock/.buzz-commitment.json"
if "$WATCHDOG" validate --config "$ROOT/config.json" >/dev/null 2>&1; then
  echo "self-targeting fallback unexpectedly passed validation" >&2
  exit 1
fi

# Version-1, missing-owner, missing-thread, and unknown-key markers all fail
# closed instead of producing noisy or unresumable alerts.
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" invalid-v1 "$ROOT/stale.heartbeat" epoch-1
jq '.version = 1' "$ROOT/watch/stale.lock/.buzz-commitment.json" > "$ROOT/marker.tmp"
mv "$ROOT/marker.tmp" "$ROOT/watch/stale.lock/.buzz-commitment.json"
if "$WATCHDOG" validate --config "$ROOT/config.json" >/dev/null 2>&1; then
  echo "version-1 marker unexpectedly passed validation" >&2
  exit 1
fi
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" missing-owner "$ROOT/stale.heartbeat" epoch-1
jq 'del(.owner_mention)' "$ROOT/watch/stale.lock/.buzz-commitment.json" > "$ROOT/marker.tmp"
mv "$ROOT/marker.tmp" "$ROOT/watch/stale.lock/.buzz-commitment.json"
if "$WATCHDOG" validate --config "$ROOT/config.json" >/dev/null 2>&1; then
  echo "ownerless marker unexpectedly passed validation" >&2
  exit 1
fi
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" missing-thread "$ROOT/stale.heartbeat" epoch-1
jq '.reply_to = ""' "$ROOT/watch/stale.lock/.buzz-commitment.json" > "$ROOT/marker.tmp"
mv "$ROOT/marker.tmp" "$ROOT/watch/stale.lock/.buzz-commitment.json"
if "$WATCHDOG" validate --config "$ROOT/config.json" >/dev/null 2>&1; then
  echo "threadless marker unexpectedly passed validation" >&2
  exit 1
fi
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" unknown-key "$ROOT/stale.heartbeat" epoch-1
jq '.unexpected = true' "$ROOT/watch/stale.lock/.buzz-commitment.json" > "$ROOT/marker.tmp"
mv "$ROOT/marker.tmp" "$ROOT/watch/stale.lock/.buzz-commitment.json"
if "$WATCHDOG" validate --config "$ROOT/config.json" >/dev/null 2>&1; then
  echo "unknown marker key unexpectedly passed validation" >&2
  exit 1
fi

echo "commitment watchdog tests passed"
