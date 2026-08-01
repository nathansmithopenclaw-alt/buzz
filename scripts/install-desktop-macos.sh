#!/usr/bin/env bash
set -euo pipefail

# Installs the verified local Buzz Desktop app at one canonical path. The
# previous app is moved to a hidden recoverable backup; nothing is deleted.
#
# Usage: ./scripts/install-desktop-macos.sh [target-triple]
# Run after `just desktop-release-build` (or `just desktop-release-build <target>`).

TARGET="${1:-aarch64-apple-darwin}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="$(cd "$REPO_ROOT" && cargo metadata --manifest-path desktop/src-tauri/Cargo.toml --format-version 1 --no-deps | node -p "JSON.parse(require('fs').readFileSync(0, 'utf8')).target_directory")"
BUILT_APP="$TARGET_DIR/$TARGET/release/bundle/macos/Buzz.app"
DEST="/Applications/Buzz.app"

if [[ ! -d "$BUILT_APP" ]]; then
    echo "Error: no built app at $BUILT_APP — run 'just desktop-release-build' first." >&2
    exit 1
fi

if pgrep -x buzz-desktop >/dev/null 2>&1; then
    echo "Error: Buzz Desktop is running. Quit it normally, then rerun this installer." >&2
    exit 2
fi

"$REPO_ROOT/scripts/verify-desktop-bundle.sh" "$TARGET"

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
BACKUP_DIR="/Applications/.buzz-backups"
mkdir -p "$BACKUP_DIR"
if [[ -d "$DEST" ]]; then
    BACKUP="$BACKUP_DIR/Buzz-$STAMP.bundle"
    echo "Moving previous canonical app to recoverable backup: $BACKUP"
    mv "$DEST" "$BACKUP"
fi

echo "Installing verified build to $DEST..."
if ! mv "$BUILT_APP" "$DEST"; then
    if [[ -n "${BACKUP:-}" && -d "$BACKUP" && ! -e "$DEST" ]]; then
        mv "$BACKUP" "$DEST"
    fi
    exit 1
fi
if ! "$REPO_ROOT/scripts/verify-desktop-bundle.sh" "$TARGET" "$DEST"; then
    FAILED="$BACKUP_DIR/Buzz-failed-$STAMP.bundle"
    mv "$DEST" "$FAILED"
    if [[ -n "${BACKUP:-}" && -d "$BACKUP" ]]; then
        mv "$BACKUP" "$DEST"
        echo "Verification failed; restored previous canonical app." >&2
    fi
    exit 1
fi

echo "Installed and verified. Launch only with: open '$DEST'"
