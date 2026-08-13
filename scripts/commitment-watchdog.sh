#!/bin/bash
# Detect opt-in, heartbeat-backed work commitments that have gone stale and
# publish one loud escalation per commitment epoch. Runtime-specific identity,
# relay, paths, and recipients live in JSON configuration/marker files; this
# script contains no deployment-specific identities.

set -uo pipefail

usage() {
  cat <<'EOF'
Usage:
  commitment-watchdog.sh validate --config <path>
  commitment-watchdog.sh check    --config <path>
  commitment-watchdog.sh status   --config <path>

Environment:
  COMMITMENT_WATCHDOG_DRY_RUN=1  write would-be alerts to state/outbox only
EOF
}

die() {
  printf 'commitment-watchdog: %s\n' "$*" >&2
  exit 1
}

MODE=""
CONFIG=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    validate|check|status)
      [ -z "$MODE" ] || die "mode specified more than once"
      MODE="$1"
      ;;
    --config)
      shift
      [ "$#" -gt 0 ] || die "--config requires a path"
      CONFIG="$1"
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *) die "unknown argument: $1" ;;
  esac
  shift
done

[ -n "$MODE" ] || { usage >&2; exit 1; }
[ -n "$CONFIG" ] || die "--config is required"
[ -f "$CONFIG" ] || die "config does not exist: $CONFIG"
command -v jq >/dev/null 2>&1 || die "jq is required"

json_required_string() {
  jq -er --arg key "$2" '.[$key] | select(type == "string" and length > 0)' "$1" 2>/dev/null
}

reject_unknown_keys() {
  local file="$1" allowed="$2" unknown
  unknown=$(jq -r --argjson allowed "$allowed" '[keys[] | select(. as $key | ($allowed | index($key) | not))] | join(",")' "$file" 2>/dev/null) \
    || die "invalid JSON: $file"
  [ -z "$unknown" ] || die "unknown key(s) in $file: $unknown"
}

reject_unknown_keys "$CONFIG" '["version","watch_root","state_root","log_path","relay_url","buzz_cli","keychain_service","expected_signer_pubkey","marker_name","claim_ttl_seconds"]'

VERSION=$(jq -er '.version | select(. == 1)' "$CONFIG" 2>/dev/null) || die "config.version must be 1"
WATCH_ROOT=$(json_required_string "$CONFIG" watch_root) || die "config.watch_root is required"
STATE_ROOT=$(json_required_string "$CONFIG" state_root) || die "config.state_root is required"
LOG_PATH=$(json_required_string "$CONFIG" log_path) || die "config.log_path is required"
RELAY_URL=$(json_required_string "$CONFIG" relay_url) || die "config.relay_url is required"
BUZZ_CLI=$(json_required_string "$CONFIG" buzz_cli) || die "config.buzz_cli is required"
KEYCHAIN_SERVICE=$(json_required_string "$CONFIG" keychain_service) || die "config.keychain_service is required"
EXPECTED_SIGNER=$(json_required_string "$CONFIG" expected_signer_pubkey) || die "config.expected_signer_pubkey is required"
MARKER_NAME=$(jq -er '.marker_name // ".buzz-commitment.json" | select(type == "string" and length > 0)' "$CONFIG" 2>/dev/null) \
  || die "config.marker_name must be a non-empty string"
CLAIM_TTL=$(jq -er '.claim_ttl_seconds // 300 | select(type == "number" and floor == . and . >= 30 and . <= 3600)' "$CONFIG" 2>/dev/null) \
  || die "config.claim_ttl_seconds must be an integer from 30 to 3600"
DRY_RUN=${COMMITMENT_WATCHDOG_DRY_RUN:-0}

case "$WATCH_ROOT:$STATE_ROOT:$LOG_PATH:$RELAY_URL:$BUZZ_CLI:$KEYCHAIN_SERVICE" in
  *$'\n'*|*$'\r'*) die "configured string values may not contain newlines" ;;
esac
printf '%s' "$MARKER_NAME" | grep -Eq '^[A-Za-z0-9._-]+$' \
  || die "config.marker_name must contain only letters, digits, dot, underscore, or hyphen"
printf '%s' "$EXPECTED_SIGNER" | grep -Eq '^[0-9a-fA-F]{64}$' || die "expected_signer_pubkey must be 64 hex characters"
[ -d "$WATCH_ROOT" ] || die "watch_root is not a directory: $WATCH_ROOT"
[ -x "$BUZZ_CLI" ] || die "buzz_cli is not executable: $BUZZ_CLI"

mkdir -p "$STATE_ROOT/claims" "$STATE_ROOT/alerts" "$STATE_ROOT/outbox" "$(dirname "$LOG_PATH")" \
  || die "could not create state/log directories"

timestamp() { date -u '+%Y-%m-%dT%H:%M:%SZ'; }
log() { printf '%s %s\n' "$(timestamp)" "$*" >> "$LOG_PATH"; }

mtime_seconds() {
  if stat -f '%m' "$1" >/dev/null 2>&1; then
    stat -f '%m' "$1"
  else
    stat -c '%Y' "$1"
  fi
}

sha256_text() {
  if command -v shasum >/dev/null 2>&1; then
    printf '%s' "$1" | shasum -a 256 | awk '{print $1}'
  else
    printf '%s' "$1" | sha256sum | awk '{print $1}'
  fi
}

iso_from_epoch() {
  if date -u -r "$1" '+%Y-%m-%dT%H:%M:%SZ' >/dev/null 2>&1; then
    date -u -r "$1" '+%Y-%m-%dT%H:%M:%SZ'
  else
    date -u -d "@$1" '+%Y-%m-%dT%H:%M:%SZ'
  fi
}

humanize_seconds() {
  local seconds="$1" years days hours minutes
  if [ "$seconds" -ge 31536000 ]; then
    years=$(( seconds / 31536000 ))
    days=$(( (seconds % 31536000) / 86400 ))
    printf '%sy %sd' "$years" "$days"
  elif [ "$seconds" -ge 86400 ]; then
    days=$(( seconds / 86400 ))
    hours=$(( (seconds % 86400) / 3600 ))
    printf '%sd %sh' "$days" "$hours"
  elif [ "$seconds" -ge 3600 ]; then
    hours=$(( seconds / 3600 ))
    minutes=$(( (seconds % 3600) / 60 ))
    printf '%sh %sm' "$hours" "$minutes"
  elif [ "$seconds" -ge 60 ]; then
    printf '%sm' "$(( seconds / 60 ))"
  else
    printf '%ss' "$seconds"
  fi
}

marker_error() {
  log "INVALID marker=$1 reason=$2"
  return 1
}

validate_marker() {
  local marker="$1" unknown expected_signer
  unknown=$(jq -r '[keys[] | select(. as $key | (["version","commitment_id","epoch","description","channel_id","reply_to","delegator_display","delegator_pubkey","fallback_mentions","stale_after_seconds","heartbeat_paths"] | index($key) | not))] | join(",")' "$marker" 2>/dev/null) \
    || { marker_error "$marker" "invalid JSON"; return 1; }
  [ -z "$unknown" ] || { marker_error "$marker" "unknown keys: $unknown"; return 1; }
  jq -e '
    .version == 1 and
    (.commitment_id | type == "string" and test("^[A-Za-z0-9._:-]{1,128}$")) and
    (.epoch | type == "string" and test("^[A-Za-z0-9._:-]{1,128}$")) and
    (.description | type == "string" and length > 0 and length <= 500 and (test("[\\r\\n@]"; "m") | not)) and
    (.channel_id | type == "string" and test("^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$")) and
    ((.reply_to // "") | type == "string" and (. == "" or test("^[0-9a-fA-F]{64}$"))) and
    (.delegator_display | type == "string" and length > 0 and length <= 100 and (test("[\\r\\n@]"; "m") | not)) and
    (.delegator_pubkey | type == "string" and test("^[0-9a-fA-F]{64}$")) and
    ((.fallback_mentions // []) | type == "array" and length <= 3 and all(.[]; type == "string" and test("^[0-9a-fA-F]{64}$"))) and
    (.stale_after_seconds | type == "number" and floor == . and . >= 60 and . <= 604800) and
    (.heartbeat_paths | type == "array" and length > 0 and length <= 16 and all(.[]; type == "string" and startswith("/") and (test("[\\r\\n]") | not)))
  ' "$marker" >/dev/null 2>&1 || { marker_error "$marker" "schema validation failed"; return 1; }
  expected_signer=$(printf '%s' "$EXPECTED_SIGNER" | tr '[:upper:]' '[:lower:]')
  if jq -e --arg signer "$expected_signer" '
    any(([.delegator_pubkey] + (.fallback_mentions // []))[]; ascii_downcase == $signer)
  ' "$marker" >/dev/null 2>&1; then
    marker_error "$marker" "configured signer must not equal an escalation mention target"
    return 1
  fi
}

freshness_for_marker() {
  local marker="$1" latest path mt
  latest=$(mtime_seconds "$marker") || return 1
  while IFS= read -r path; do
    [ -e "$path" ] || continue
    mt=$(mtime_seconds "$path") || continue
    [ "$mt" -le "$latest" ] || latest="$mt"
  done < <(jq -r '.heartbeat_paths[]' "$marker")
  printf '%s\n' "$latest"
}

claim_alert() {
  local action_key="$1" claim="$STATE_ROOT/claims/$action_key" started age stale_path
  if mkdir "$claim" 2>/dev/null; then
    printf '%s\n' "$(date +%s)" > "$claim/started"
    return 0
  fi
  started=$(cat "$claim/started" 2>/dev/null || printf '0')
  case "$started" in ''|*[!0-9]*) started=0 ;; esac
  age=$(( $(date +%s) - started ))
  if [ "$age" -gt "$CLAIM_TTL" ]; then
    stale_path="$claim.stale.$$.$(date +%s)"
    if mv "$claim" "$stale_path" 2>/dev/null && mkdir "$claim" 2>/dev/null; then
      printf '%s\n' "$(date +%s)" > "$claim/started"
      log "broke stale alert claim action=$action_key age=${age}s"
      return 0
    fi
  fi
  return 1
}

release_claim() {
  local claim="$STATE_ROOT/claims/$1"
  rm -f "$claim/started"
  rmdir "$claim" 2>/dev/null || true
}

read_signing_key() {
  security find-generic-password -s "$KEYCHAIN_SERVICE" -w 2>>"$LOG_PATH"
}

signer_is_expected() {
  local key="$1" profile actual expected
  profile=$(env BUZZ_RELAY_URL="$RELAY_URL" BUZZ_PRIVATE_KEY="$key" BUZZ_AUTH_TAG= \
    "$BUZZ_CLI" users get 2>>"$LOG_PATH") || return 1
  actual=$(printf '%s' "$profile" | jq -er '.[0].pubkey' 2>/dev/null) || return 1
  actual=$(printf '%s' "$actual" | tr '[:upper:]' '[:lower:]')
  expected=$(printf '%s' "$EXPECTED_SIGNER" | tr '[:upper:]' '[:lower:]')
  [ "$actual" = "$expected" ]
}

relay_has_action() {
  local key="$1" channel="$2" action_key="$3" found expected
  found=$(env BUZZ_RELAY_URL="$RELAY_URL" BUZZ_PRIVATE_KEY="$key" BUZZ_AUTH_TAG= \
    "$BUZZ_CLI" messages search --author "$EXPECTED_SIGNER" --query "$action_key" --limit 20 2>>"$LOG_PATH") || return 2
  expected=$(printf '%s' "$EXPECTED_SIGNER" | tr '[:upper:]' '[:lower:]')
  printf '%s' "$found" | jq -e --arg signer "$expected" --arg channel "$channel" --arg token "commitment-watch:$action_key" '
    any(.[]; ((.pubkey // "") | ascii_downcase) == $signer and (.content // "" | contains($token)) and any(.tags[]?; .[0] == "h" and .[1] == $channel))
  ' >/dev/null 2>&1
}

write_alert_state() {
  local action_key="$1" status="$2" event_id="${3:-}" destination="$STATE_ROOT/alerts/$action_key.json" tmp
  tmp="$destination.tmp.$$"
  jq -n --arg status "$status" --arg event_id "$event_id" --arg observed_at "$(timestamp)" \
    '{status:$status,event_id:(if $event_id == "" then null else $event_id end),observed_at:$observed_at}' > "$tmp" \
    && mv "$tmp" "$destination"
}

send_alert() {
  local marker="$1" action_key="$2" last_heartbeat="$3" age="$4"
  local channel reply_to display delegator description key output event_id body relay_status
  local -a args mentions

  channel=$(jq -r '.channel_id' "$marker")
  reply_to=$(jq -r '.reply_to // ""' "$marker")
  display=$(jq -r '.delegator_display' "$marker")
  delegator=$(jq -r '.delegator_pubkey' "$marker")
  description=$(jq -r '.description' "$marker")
  mentions=("$delegator")
  while IFS= read -r fallback; do mentions+=("$fallback"); done < <(jq -r '.fallback_mentions[]?' "$marker")

  body="@$display COMMITMENT STALLED (automated watchdog): $description has no declared heartbeat for $(humanize_seconds "$age"). The owning session may have died after committing to work; inspect, resume, or explicitly close the commitment.

Last heartbeat: $(iso_from_epoch "$last_heartbeat")
commitment-watch:$action_key

[Signed by the configured watchdog identity; this alert is emitted at most once for this commitment epoch.]"

  if [ "$DRY_RUN" = "1" ]; then
    printf '%s\n' "$body" > "$STATE_ROOT/outbox/$action_key.md"
    write_alert_state "$action_key" dry_run
    log "DRY_RUN alert action=$action_key channel=$channel"
    return 0
  fi

  key=$(read_signing_key) || { log "ERROR no signing key service=$KEYCHAIN_SERVICE"; return 1; }
  [ -n "$key" ] || { log "ERROR empty signing key service=$KEYCHAIN_SERVICE"; return 1; }
  if ! signer_is_expected "$key"; then
    log "ERROR signing key did not resolve to expected signer=$EXPECTED_SIGNER"
    return 1
  fi

  if relay_has_action "$key" "$channel" "$action_key"; then
    write_alert_state "$action_key" observed_on_relay
    log "deduped action already present on relay action=$action_key"
    return 0
  else
    relay_status=$?
    if [ "$relay_status" -eq 2 ]; then
      # A failed lookup is ambiguous: a prior process may have published before
      # crashing without durable local state. Fail closed and retry later rather
      # than risk a duplicate escalation.
      log "ERROR relay dedupe query failed; refusing ambiguous send action=$action_key"
      return 1
    fi
  fi

  args=(messages send --channel "$channel" --content -)
  [ -z "$reply_to" ] || args+=(--reply-to "$reply_to")
  local mention
  for mention in "${mentions[@]}"; do args+=(--mention "$mention"); done
  output=$(printf '%s\n' "$body" | env BUZZ_RELAY_URL="$RELAY_URL" BUZZ_PRIVATE_KEY="$key" BUZZ_AUTH_TAG= \
    "$BUZZ_CLI" "${args[@]}" 2>>"$LOG_PATH") || { log "ERROR relay send failed action=$action_key"; return 1; }
  event_id=$(printf '%s' "$output" | jq -er 'select(.accepted == true) | .event_id' 2>/dev/null) \
    || { log "ERROR relay send returned no accepted event action=$action_key"; return 1; }
  write_alert_state "$action_key" sent "$event_id"
  log "posted action=$action_key event=$event_id signer=$EXPECTED_SIGNER channel=$channel"
}

check_marker() {
  local marker="$1" commitment epoch action_key alert_state alert_status latest now age stale_after
  validate_marker "$marker" || return 1
  commitment=$(jq -r '.commitment_id' "$marker")
  epoch=$(jq -r '.epoch' "$marker")
  action_key=$(sha256_text "$commitment|$epoch")
  alert_state="$STATE_ROOT/alerts/$action_key.json"
  if [ -f "$alert_state" ]; then
    alert_status=$(jq -r '.status // "invalid"' "$alert_state" 2>/dev/null)
    if [ "$alert_status" != "dry_run" ] || [ "$DRY_RUN" = "1" ]; then
      log "suppressed previously recorded action=$action_key status=$alert_status marker=$marker"
      return 0
    fi
    log "ignoring dry-run-only state for production action=$action_key marker=$marker"
  fi
  latest=$(freshness_for_marker "$marker") || { marker_error "$marker" "could not read freshness"; return 1; }
  now=$(date +%s)
  age=$(( now - latest ))
  stale_after=$(jq -r '.stale_after_seconds' "$marker")
  if [ "$age" -lt "$stale_after" ]; then
    log "healthy commitment=$commitment epoch=$epoch age=${age}s threshold=${stale_after}s"
    return 0
  fi
  if ! claim_alert "$action_key"; then
    log "suppressed active alert claim action=$action_key"
    return 0
  fi

  # Close the mtime/read-to-post race: the worker may have heartbeated while we
  # were claiming the action. Recompute after the claim and suppress if healthy.
  latest=$(freshness_for_marker "$marker") || { release_claim "$action_key"; return 1; }
  now=$(date +%s)
  age=$(( now - latest ))
  if [ "$age" -lt "$stale_after" ]; then
    log "recovered before post commitment=$commitment action=$action_key age=${age}s"
    release_claim "$action_key"
    return 0
  fi

  if send_alert "$marker" "$action_key" "$latest" "$age"; then
    release_claim "$action_key"
    return 0
  fi
  release_claim "$action_key"
  return 1
}

list_markers() {
  find "$WATCH_ROOT" -maxdepth 3 -type f -name "$MARKER_NAME" -print 2>/dev/null | LC_ALL=C sort
}

case "$MODE" in
  validate)
    count=0
    while IFS= read -r marker; do
      [ -n "$marker" ] || continue
      validate_marker "$marker" || exit 1
      count=$((count + 1))
    done < <(list_markers)
    printf 'config valid; %s commitment marker(s) valid\n' "$count"
    ;;
  status)
    while IFS= read -r marker; do
      [ -n "$marker" ] || continue
      if validate_marker "$marker"; then
        latest=$(freshness_for_marker "$marker")
        printf '%s\t%s\tage=%ss\tthreshold=%ss\n' \
          "$(jq -r '.commitment_id' "$marker")" "$(jq -r '.epoch' "$marker")" \
          "$(( $(date +%s) - latest ))" "$(jq -r '.stale_after_seconds' "$marker")"
      fi
    done < <(list_markers)
    ;;
  check)
    failures=0
    while IFS= read -r marker; do
      [ -n "$marker" ] || continue
      check_marker "$marker" || failures=$((failures + 1))
    done < <(list_markers)
    [ "$failures" -eq 0 ] || exit 1
    ;;
esac
