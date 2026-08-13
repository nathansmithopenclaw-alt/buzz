#!/bin/bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
WATCHDOG="$SCRIPT_DIR/commitment-watchdog.sh"
ROOT=$(mktemp -d)
trap 'rm -rf "$ROOT"' EXIT

mkdir -p "$ROOT/watch/healthy.lock" "$ROOT/watch/stale.lock" "$ROOT/state" "$ROOT/bin"
touch "$ROOT/healthy.heartbeat" "$ROOT/stale.heartbeat"

cat > "$ROOT/bin/buzz" <<'SH'
#!/bin/bash
set -eu
case "${1:-} ${2:-}" in
  "users get")
    pubkey=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
    [ ! -f "$MOCK_BUZZ_STATE/bad-signer" ] || pubkey=dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd
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
    content=$(cat)
    printf '%s\n' "$content" > "$MOCK_BUZZ_STATE/last-message.md"
    count=$(cat "$MOCK_BUZZ_STATE/send-count" 2>/dev/null || printf '0')
    count=$((count + 1))
    printf '%s\n' "$count" > "$MOCK_BUZZ_STATE/send-count"
    jq -n --arg content "$content" '[{pubkey:"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",content:$content,tags:[["h","00000000-0000-0000-0000-000000000000"]]}]' > "$MOCK_BUZZ_STATE/relay-event.json"
    printf '{"accepted":true,"event_id":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}\n'
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
  "keychain_service": "unused-in-dry-run",
  "expected_signer_pubkey": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "marker_name": ".buzz-commitment.json",
  "claim_ttl_seconds": 30
}
JSON

write_marker() {
  local path="$1" id="$2" heartbeat="$3" epoch="$4"
  cat > "$path" <<JSON
{
  "version": 1,
  "commitment_id": "$id",
  "epoch": "$epoch",
  "description": "$id proof commitment",
  "channel_id": "00000000-0000-0000-0000-000000000000",
  "reply_to": "",
  "delegator_display": "Test Orchestrator",
  "delegator_pubkey": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  "fallback_mentions": [],
  "stale_after_seconds": 60,
  "heartbeat_paths": ["$heartbeat"]
}
JSON
}

write_marker "$ROOT/watch/healthy.lock/.buzz-commitment.json" healthy "$ROOT/healthy.heartbeat" epoch-1
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" stale "$ROOT/stale.heartbeat" epoch-1

"$WATCHDOG" validate --config "$ROOT/config.json" | grep -q '2 commitment marker(s) valid'

# Healthy work remains quiet even when the lock/marker itself is old: the
# declared heartbeat, not elapsed wall time alone, is the liveness contract.
touch -t 202001010000 "$ROOT/watch/healthy.lock/.buzz-commitment.json"
touch "$ROOT/healthy.heartbeat"
touch -t 202001010000 "$ROOT/watch/stale.lock/.buzz-commitment.json" "$ROOT/stale.heartbeat"
COMMITMENT_WATCHDOG_DRY_RUN=1 "$WATCHDOG" check --config "$ROOT/config.json"

outbox_count=$(find "$ROOT/state/outbox" -type f | wc -l | tr -d ' ')
[ "$outbox_count" = 1 ]
grep -q 'stale proof commitment' "$ROOT/state/outbox"/*.md
! grep -q 'healthy proof commitment' "$ROOT/state/outbox"/*.md
grep -Eq 'no declared heartbeat for [0-9]+y [0-9]+d' "$ROOT/state/outbox"/*.md

# Rechecking the same stale epoch is at-most-once.
COMMITMENT_WATCHDOG_DRY_RUN=1 "$WATCHDOG" check --config "$ROOT/config.json"
outbox_count=$(find "$ROOT/state/outbox" -type f | wc -l | tr -d ' ')
[ "$outbox_count" = 1 ]

# A fresh heartbeat before a new epoch suppresses the new action.
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" stale "$ROOT/stale.heartbeat" epoch-2
touch "$ROOT/stale.heartbeat"
COMMITMENT_WATCHDOG_DRY_RUN=1 "$WATCHDOG" check --config "$ROOT/config.json"
outbox_count=$(find "$ROOT/state/outbox" -type f | wc -l | tr -d ' ')
[ "$outbox_count" = 1 ]

# A dry-run receipt is evidence only: it must not poison the same epoch when
# the production watcher subsequently runs.
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" stale "$ROOT/stale.heartbeat" dry-then-live
touch -t 202001010000 "$ROOT/watch/stale.lock/.buzz-commitment.json" "$ROOT/stale.heartbeat"
COMMITMENT_WATCHDOG_DRY_RUN=1 "$WATCHDOG" check --config "$ROOT/config.json"
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 1 ]

# The configured signing-key boundary fails closed if the key resolves to any
# identity other than the explicitly expected, owner-attested signer.
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" stale "$ROOT/stale.heartbeat" wrong-signer
touch -t 202001010000 "$ROOT/watch/stale.lock/.buzz-commitment.json" "$ROOT/stale.heartbeat"
touch "$MOCK_BUZZ_STATE/bad-signer"
if "$WATCHDOG" check --config "$ROOT/config.json"; then
  echo "unexpected signer was allowed to publish" >&2
  exit 1
fi
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 1 ]
rm -f "$MOCK_BUZZ_STATE/bad-signer"

# Exercise the production signer, pre-send relay dedupe, accepted-send, and
# crash-window recovery paths with a fake CLI/keychain boundary.
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" stale "$ROOT/stale.heartbeat" production-1
touch -t 202001010000 "$ROOT/watch/stale.lock/.buzz-commitment.json" "$ROOT/stale.heartbeat"
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 2 ]
grep -q 'commitment-watch:' "$MOCK_BUZZ_STATE/last-message.md"

# Simulate a crash after relay acceptance but before local state persistence.
# The relay token must recover the action without a duplicate send.
if command -v shasum >/dev/null 2>&1; then
  production_key=$(printf '%s' 'stale|production-1' | shasum -a 256 | awk '{print $1}')
else
  production_key=$(printf '%s' 'stale|production-1' | sha256sum | awk '{print $1}')
fi
rm -f "$ROOT/state/alerts/$production_key.json"
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 2 ]
jq -e '.status == "observed_on_relay"' "$ROOT/state/alerts/$production_key.json" >/dev/null

# A failed dedupe query is ambiguous and must fail closed. Once lookup health
# returns, the pending new epoch is delivered normally.
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" stale "$ROOT/stale.heartbeat" production-2
touch -t 202001010000 "$ROOT/watch/stale.lock/.buzz-commitment.json" "$ROOT/stale.heartbeat"
touch "$MOCK_BUZZ_STATE/fail-search"
if "$WATCHDOG" check --config "$ROOT/config.json"; then
  echo "ambiguous relay lookup unexpectedly allowed the check" >&2
  exit 1
fi
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 2 ]
rm -f "$MOCK_BUZZ_STATE/fail-search"
"$WATCHDOG" check --config "$ROOT/config.json"
[ "$(cat "$MOCK_BUZZ_STATE/send-count")" = 3 ]

# A signer can never mention itself: those events are delivered to the relay
# but cannot wake the same identity. Reject the marker before it can silently
# collapse the escalation path.
jq '.delegator_pubkey = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"' \
  "$ROOT/watch/stale.lock/.buzz-commitment.json" > "$ROOT/self-target.json"
mv "$ROOT/self-target.json" "$ROOT/watch/stale.lock/.buzz-commitment.json"
if "$WATCHDOG" validate --config "$ROOT/config.json" >/dev/null 2>&1; then
  echo "self-targeting signer unexpectedly passed validation" >&2
  exit 1
fi

# Invalid or underspecified markers fail validation instead of creating noisy
# or misaddressed alerts.
write_marker "$ROOT/watch/stale.lock/.buzz-commitment.json" stale "$ROOT/stale.heartbeat" invalid-marker
jq '.unexpected = true' "$ROOT/watch/stale.lock/.buzz-commitment.json" > "$ROOT/invalid.json"
mv "$ROOT/invalid.json" "$ROOT/watch/stale.lock/.buzz-commitment.json"
if "$WATCHDOG" validate --config "$ROOT/config.json" >/dev/null 2>&1; then
  echo "invalid marker unexpectedly passed" >&2
  exit 1
fi

echo "commitment watchdog tests passed"
