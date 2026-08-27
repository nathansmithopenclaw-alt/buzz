#!/usr/bin/env bash
# =============================================================================
# start-isolated-test-relay.sh — GUI read-model overhaul test harness (Dawn)
# =============================================================================
# Stands up a FULLY ISOLATED relay for seeding + parity/perf runs, from source
# on the current branch. Never touches the shared :3000 team relay or the
# default `buzz-*` dev stack. Backing services run under the dedicated
# `buzz-harness` Compose project (docker-compose.harness.yml); the relay runs
# in the foreground on override ports.
#
#   Topology (reuse this exact tuple for desktop parity runs):
#     compose project : buzz-harness
#     postgres        : localhost:5471  (db=buzz, user=buzz, pass=buzz_dev)
#     redis           : localhost:6471
#     minio           : localhost:9471 (console 9472)
#     relay main      : localhost:3030   ← BUZZ_E2E_RELAY_URL=http://localhost:3030
#     relay health    : localhost:8088
#     relay metrics   : localhost:9202
#
# Usage:
#   BUZZ_HARNESS_RELAY_PORT=3031 BUZZ_HARNESS_DB_NAME=buzz_lane \
#     ./scripts/start-isolated-test-relay.sh [--profile <cargo-profile>]
#
# Shared-services teardown (only when no other harness lanes are active):
#   docker compose -p buzz-harness -f docker-compose.harness.yml down -v
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

CARGO_PROFILE="${CARGO_PROFILE:-ci}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile) CARGO_PROFILE="$2"; shift 2 ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done

# Cargo names the development profile `dev`, but writes its binaries under
# target/debug. Accept `debug` as the user-facing spelling too.
case "${CARGO_PROFILE}" in
  dev|debug)
    CARGO_BUILD_PROFILE="dev"
    CARGO_TARGET_PROFILE="debug"
    ;;
  *)
    CARGO_BUILD_PROFILE="${CARGO_PROFILE}"
    CARGO_TARGET_PROFILE="${CARGO_PROFILE}"
    ;;
esac

PROJECT="buzz-harness"
COMPOSE_FILE="docker-compose.harness.yml"

# Isolated ports (distinct from :3000 team relay, default dev stack, and Eva's
# evaperf :5470/:6470/:9470/:3170 stack).
PG_PORT=5471
REDIS_PORT=6471
MINIO_PORT=9471
RELAY_MAIN="${BUZZ_HARNESS_RELAY_PORT:-3030}"
DB_NAME="${BUZZ_HARNESS_DB_NAME:-buzz}"

if [[ ! "${RELAY_MAIN}" =~ ^[0-9]+$ ]] || (( RELAY_MAIN < 1024 || RELAY_MAIN > 65535 )); then
  echo "BUZZ_HARNESS_RELAY_PORT must be an unprivileged TCP port" >&2
  exit 1
fi
if [[ ! "${DB_NAME}" =~ ^[a-zA-Z_][a-zA-Z0-9_]*$ ]]; then
  echo "BUZZ_HARNESS_DB_NAME must be a PostgreSQL identifier" >&2
  exit 1
fi
RELAY_HEALTH="${BUZZ_HARNESS_HEALTH_PORT:-$((RELAY_MAIN + 5058))}"
RELAY_METRICS="${BUZZ_HARNESS_METRICS_PORT:-$((RELAY_MAIN + 6172))}"
COMMUNITY_HOST="localhost:${RELAY_MAIN}"
for port in "${RELAY_HEALTH}" "${RELAY_METRICS}"; do
  if [[ ! "${port}" =~ ^[0-9]+$ ]] || (( port < 1024 || port > 65535 )); then
    echo "derived or overridden harness ports must be unprivileged TCP ports" >&2
    exit 1
  fi
done

BLUE='\033[0;34m'; GREEN='\033[0;32m'; RED='\033[0;31m'; NC='\033[0m'
log() { echo -e "${BLUE}[isolated-relay]${NC} $*"; }
ok()  { echo -e "${GREEN}[isolated-relay]${NC} $*"; }
err() { echo -e "${RED}[isolated-relay]${NC} $*" >&2; }

# Refuse before starting or resetting any shared harness service. A second
# lane may already own :3030 and the buzz-harness database behind it; checking
# only after schema reset destroys that lane's isolated test state.
if command -v lsof >/dev/null 2>&1; then
  for port in "${RELAY_MAIN}" "${RELAY_HEALTH}" "${RELAY_METRICS}"; do
    if lsof -nP -iTCP:"${port}" -sTCP:LISTEN >/dev/null 2>&1; then
      err "Port ${port} is already in use; refusing before buzz-harness reset."
      lsof -nP -iTCP:"${port}" -sTCP:LISTEN >&2 || true
      exit 1
    fi
  done
fi

# ── Backing services (scoped to buzz-harness only) ───────────────────────────
log "Bringing up backing services (project=${PROJECT})..."
docker compose -p "${PROJECT}" -f "${COMPOSE_FILE}" up -d

wait_pg() {
  for _ in $(seq 1 60); do
    if docker compose -p "${PROJECT}" -f "${COMPOSE_FILE}" exec -T postgres \
         pg_isready -U buzz >/dev/null 2>&1; then
      ok "Postgres ready"; return 0
    fi
    sleep 2
  done
  err "Postgres did not become ready"; return 1
}
wait_pg

# ── Schema + partitions ──────────────────────────────────────────────────────
export PGPASSWORD=buzz_dev
psql_h() { docker compose -p "${PROJECT}" -f "${COMPOSE_FILE}" exec -T postgres \
  psql -U buzz -d "${DB_NAME}" -v ON_ERROR_STOP=1 "$@"; }

log "Ensuring isolated database ${DB_NAME} exists..."
if ! docker compose -p "${PROJECT}" -f "${COMPOSE_FILE}" exec -T postgres \
     psql -U buzz -d postgres -tAc "SELECT 1 FROM pg_database WHERE datname = '${DB_NAME}'" \
     | grep -qx 1; then
  docker compose -p "${PROJECT}" -f "${COMPOSE_FILE}" exec -T postgres \
    createdb -U buzz "${DB_NAME}"
fi

log "Resetting isolated database ${DB_NAME} and applying schema..."
# This database belongs only to the buzz-harness Compose project. Reset it on
# every launch so stale partitions/events from an earlier proof cannot alter
# schema planning or test results.
psql_h -c "DROP SCHEMA public CASCADE; CREATE SCHEMA public;"
# Extensions are database-scoped. Per-lane databases do not inherit the
# default harness database's pgcrypto installation, and the journal hash
# functions require digest(bytea, text).
psql_h -c "CREATE EXTENSION IF NOT EXISTS pgcrypto;"
export PGSCHEMA_PLAN_HOST=localhost PGSCHEMA_PLAN_PORT=${PG_PORT}
export PGSCHEMA_PLAN_DB="${DB_NAME}" PGSCHEMA_PLAN_USER=buzz PGSCHEMA_PLAN_PASSWORD=buzz_dev
export PGHOST=localhost PGPORT=${PG_PORT} PGUSER=buzz PGDATABASE="${DB_NAME}"
./bin/pgschema apply --file schema/schema.sql --auto-approve
psql_h < scripts/reconcile-schema-after-pgschema.sql
ok "Schema applied"

# ── Deployment community + channels + members ────────────────────────────────
# setup-desktop-test-data.sh is the single writer of the dev community row and
# the channel/member seed. It keys everything off a fixed COMMUNITY_ID and an
# overridable host — point that host at OUR relay so the tenant binding matches,
# and point its DB env at OUR isolated postgres. (psql is on PATH, so it uses
# BUZZ_DB_HOST/PORT rather than the shared `buzz-postgres` container.)
log "Seeding community (host=${COMMUNITY_HOST}), channels, and members..."
BUZZ_COMMUNITY_HOST="${COMMUNITY_HOST}" \
  BUZZ_DB_HOST=localhost BUZZ_DB_PORT=${PG_PORT} BUZZ_DB_USER=buzz \
  BUZZ_DB_PASS=buzz_dev BUZZ_DB_NAME="${DB_NAME}" \
  BUZZ_DB_DOCKER_CONTAINER="${PROJECT}-postgres-1" \
  ./scripts/setup-desktop-test-data.sh
ok "Community + channels + members seeded"

# ── Build relay from source (current branch) ─────────────────────────────────
# The repo pins Rust via rust-toolchain.toml (1.95.0). Outside the hermit env a
# stray Homebrew `cargo` (1.89) shadows the pin and fails on sqlx's MSRV, so
# prefer the rustup shim, which honors the pin.
if [[ -x "${HOME}/.cargo/bin/cargo" ]]; then
  export PATH="${HOME}/.cargo/bin:${PATH}"
fi
log "Building relay (profile=${CARGO_BUILD_PROFILE}, cargo=$(command -v cargo), $(cargo --version))..."
cargo build --profile "${CARGO_BUILD_PROFILE}" -p buzz-relay
ok "Relay built"

# ── Run relay (detached tmux session) ────────────────────────────────────────
# Run inside tmux, NOT the foreground: this script is invoked from ephemeral
# shells whose process group is reaped on return, which SIGTERMs a foreground
# relay ~seconds after startup. tmux fully daemonizes the session so the relay
# survives (same pattern the perf stack uses). Logs to ${RELAY_LOG}.
if [[ "${RELAY_MAIN}" == "3030" ]]; then
  DEFAULT_RELAY_LOG=/tmp/dawn-relay-run.log
  DEFAULT_TMUX_SESSION=dawn-relay
else
  DEFAULT_RELAY_LOG="/tmp/dawn-relay-${RELAY_MAIN}-run.log"
  DEFAULT_TMUX_SESSION="dawn-relay-${RELAY_MAIN}"
fi
RELAY_LOG="${RELAY_LOG:-${DEFAULT_RELAY_LOG}}"
TMUX_SESSION="${TMUX_SESSION:-${DEFAULT_TMUX_SESSION}}"
RELAY_PRIVATE_KEY="$(openssl rand -hex 32)"
tmux kill-session -t "${TMUX_SESSION}" 2>/dev/null || true
if command -v lsof >/dev/null 2>&1 && lsof -nP -iTCP:"${RELAY_MAIN}" -sTCP:LISTEN >/dev/null 2>&1; then
  err "Port ${RELAY_MAIN} is already in use; refusing to report a stale relay as this harness."
  lsof -nP -iTCP:"${RELAY_MAIN}" -sTCP:LISTEN >&2 || true
  exit 1
fi
log "Starting relay in tmux session '${TMUX_SESSION}' on :${RELAY_MAIN} (health :${RELAY_HEALTH}, metrics :${RELAY_METRICS})..."
tmux new-session -d -s "${TMUX_SESSION}" "cd '${REPO_ROOT}' && env \
  DATABASE_URL=postgres://buzz:buzz_dev@localhost:${PG_PORT}/${DB_NAME} \
  REDIS_URL=redis://localhost:${REDIS_PORT} \
  RELAY_URL=ws://localhost:${RELAY_MAIN} \
  BUZZ_BIND_ADDR=0.0.0.0:${RELAY_MAIN} \
  BUZZ_HEALTH_PORT=${RELAY_HEALTH} \
  BUZZ_METRICS_PORT=${RELAY_METRICS} \
  BUZZ_S3_ENDPOINT=http://localhost:${MINIO_PORT} \
  BUZZ_S3_ACCESS_KEY=buzz_dev \
  BUZZ_S3_SECRET_KEY=buzz_dev_secret \
  BUZZ_S3_BUCKET=buzz-media \
  BUZZ_RELAY_PRIVATE_KEY=${RELAY_PRIVATE_KEY} \
  BUZZ_REQUIRE_AUTH_TOKEN=false \
  BUZZ_RECONCILE_CHANNELS=true \
  './target/${CARGO_TARGET_PROFILE}/buzz-relay' > '${RELAY_LOG}' 2>&1"

# Wait for the main port to accept connections.
for _ in $(seq 1 30); do
  if curl -s -o /dev/null "http://localhost:${RELAY_MAIN}/"; then
    ok "Relay live — BUZZ_E2E_RELAY_URL=http://localhost:${RELAY_MAIN}"
    ok "Logs: ${RELAY_LOG}   Attach: tmux attach -t ${TMUX_SESSION}"
    ok "Stop relay: tmux kill-session -t ${TMUX_SESSION}"
    ok "Shared-services teardown (only when no other lanes run): docker compose -p ${PROJECT} -f ${COMPOSE_FILE} down -v"
    exit 0
  fi
  sleep 1
done
err "Relay did not come up on :${RELAY_MAIN} within 30s — check ${RELAY_LOG}"
exit 1
