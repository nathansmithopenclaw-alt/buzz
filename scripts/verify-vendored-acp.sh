#!/usr/bin/env bash
# Rollout-battery runtime assertion for the vendored Track-A adapter (L4.1).
# Proves the INSTALLED artifact — not a dev checkout — is the verified build:
#   1. committed tarball still matches PROVENANCE.json sha256
#   2. npm-installed package version matches the pin
#   3. the REAL consumer, pointed at THIS vendored dist, passes the M3 live
#      typed battery (init capability -> typed-confirmed promotion -> settle)
# Any silence or fallback here is a FAIL. Requires: node, cargo, real claude CLI.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
V="$REPO_ROOT/vendor/acp-adapter"
PROV="$V/PROVENANCE.json"
TARBALL="$(python3 -c "import json;print(json.load(open('$PROV'))['tarball'])")"
EXPECT_SHA="$(python3 -c "import json;print(json.load(open('$PROV'))['tarball_sha256'])")"
EXPECT_VER="$(python3 -c "import json;print(json.load(open('$PROV'))['version'])")"
PKG="$V/node_modules/@agentclientprotocol/claude-agent-acp"

ACTUAL_SHA="$(shasum -a 256 "$V/$TARBALL" | awk '{print $1}')"
[[ "$ACTUAL_SHA" == "$EXPECT_SHA" ]] || { echo "FAIL: tarball hash drift ($ACTUAL_SHA != $EXPECT_SHA)" >&2; exit 1; }
echo "1/3 tarball hash OK"

[[ -d "$PKG" ]] || { echo "FAIL: not installed — run scripts/install-vendored-acp-adapter.sh first" >&2; exit 1; }
ACTUAL_VER="$(node -p "require('$PKG/package.json').version")"
[[ "$ACTUAL_VER" == "$EXPECT_VER" ]] || { echo "FAIL: installed $ACTUAL_VER != pinned $EXPECT_VER" >&2; exit 1; }
echo "2/3 installed version OK ($ACTUAL_VER)"

# Scrub ambient BUZZ_ACP_* (config-default tests break on ambient vars) using a
# safe read-loop, never unquoted expansion (the known word-split trap).
while IFS= read -r key; do unset "$key"; done < <(env | sed -n 's/^\(BUZZ_ACP_[A-Za-z0-9_]*\)=.*/\1/p')
CLAUDE="$(command -v claude)" || { echo "FAIL: claude CLI not on PATH" >&2; exit 1; }

cd "$REPO_ROOT"
ACP_FORK_DIST="$PKG/dist" CLAUDE_CODE_EXECUTABLE="$CLAUDE" \
    cargo test -p buzz-acp --lib m3_live_typed_configured_end_to_end -- --ignored --nocapture
echo "3/3 runtime typed battery GREEN against vendored dist: $PKG/dist"
echo "VENDORED ADAPTER VERIFIED: version $ACTUAL_VER, sha256 ${ACTUAL_SHA:0:12}…"
