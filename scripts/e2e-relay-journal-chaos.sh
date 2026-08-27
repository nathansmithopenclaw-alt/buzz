#!/usr/bin/env bash
# Restore/split-brain proof for the per-community relay event journal.
#
# Prerequisite: start the repository's isolated harness. This script refuses
# to target anything except the selected localhost harness port and named
# database on localhost:5471, then runs the integrity cases.
set -euo pipefail

RELAY_PORT="${BUZZ_HARNESS_RELAY_PORT:-3030}"
DB_NAME="${BUZZ_HARNESS_DB_NAME:-buzz}"
EXPECTED_RELAY_URL="http://localhost:${RELAY_PORT}"
EXPECTED_DATABASE_URL="postgres://buzz:buzz_dev@localhost:5471/${DB_NAME}"
RELAY_URL="${BUZZ_E2E_RELAY_URL:-${EXPECTED_RELAY_URL}}"
DATABASE_URL="${BUZZ_TEST_DATABASE_URL:-${EXPECTED_DATABASE_URL}}"

if [[ ! "${RELAY_PORT}" =~ ^[0-9]+$ ]] || (( RELAY_PORT < 1024 || RELAY_PORT > 65535 )); then
  echo "BUZZ_HARNESS_RELAY_PORT must be an unprivileged TCP port" >&2
  exit 1
fi
if [[ ! "${DB_NAME}" =~ ^[a-zA-Z_][a-zA-Z0-9_]*$ ]]; then
  echo "BUZZ_HARNESS_DB_NAME must be a PostgreSQL identifier" >&2
  exit 1
fi
if [[ "${RELAY_URL}" != "${EXPECTED_RELAY_URL}" ]]; then
  echo "refusing non-isolated relay: ${RELAY_URL}" >&2
  exit 1
fi
if [[ "${DATABASE_URL}" != "${EXPECTED_DATABASE_URL}" ]]; then
  echo "refusing non-isolated database: ${DATABASE_URL}" >&2
  exit 1
fi
if ! curl -fsS "${RELAY_URL}/health" >/dev/null; then
  echo "isolated relay is not healthy at ${RELAY_URL}; run scripts/start-isolated-test-relay.sh first" >&2
  exit 1
fi

unset BUZZ_AUTH_TAG BUZZ_RELAY_URL BUZZ_PRIVATE_KEY
export BUZZ_TEST_DATABASE_URL="${DATABASE_URL}"

cargo test -p buzz-db 'journal::tests::' -- --ignored --nocapture
