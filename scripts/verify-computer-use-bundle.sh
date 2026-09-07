#!/usr/bin/env bash
# Exercise packaged dependencies after relocation, not source-tree resolution.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
APP=${1:?Pass the packaged Buzz.app path}
STAGE=$(mktemp -d "${TMPDIR:-/tmp}/buzz-computer-relocation.XXXXXX")
trap 'rm -rf "$STAGE"' EXIT
cp -R "$APP/Contents/Resources/computer-use" "$STAGE/runtime"
cd "$ROOT"
BUZZ_TEST_COMPUTER_SCRIPT="$STAGE/runtime/src/main.mjs" \
BUZZ_TEST_COMPUTER_CLI="$APP/Contents/MacOS/buzz" \
  node --test --test-name-pattern='standard MCP|killing an owned' computer-use/test/browser.test.mjs
