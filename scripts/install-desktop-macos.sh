#!/usr/bin/env bash
set -euo pipefail

# Installs the locally built Buzz Desktop app to a single canonical
# /Applications/Buzz.app, replacing any prior copy in place.
#
# Without this, repeatedly dragging a freshly built .app into /Applications
# by hand hits macOS's name-collision auto-rename and leaves "Buzz 2.app",
# "Buzz 3.app", etc. behind — all sharing the same bundle id, none of them
# ever cleaned up. This script always replaces instead of adding.
#
# Usage: ./scripts/install-desktop-macos.sh [target-triple]
# Run after `just desktop-release-build` (or `just desktop-release-build <target>`).

TARGET="${1:-aarch64-apple-darwin}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILT_APP="$REPO_ROOT/desktop/src-tauri/target/$TARGET/release/bundle/macos/Buzz.app"
DEST="/Applications/Buzz.app"

if [[ ! -d "$BUILT_APP" ]]; then
    echo "Error: no built app at $BUILT_APP — run 'just desktop-release-build' first." >&2
    exit 1
fi

echo "Quitting any running Buzz Desktop instances..."
pkill -x buzz-desktop 2>/dev/null || true
sleep 1

echo "Removing existing /Applications/Buzz*.app copies..."
find /Applications -maxdepth 1 -iname "Buzz*.app" -exec rm -rf {} +

echo "Installing fresh build to $DEST..."
ditto "$BUILT_APP" "$DEST"

echo "Installed. Launch with: open '$DEST'"
