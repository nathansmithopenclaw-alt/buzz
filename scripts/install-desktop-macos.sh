#!/usr/bin/env bash
set -euo pipefail

# Installs the verified local Buzz Desktop app at one canonical path. The
# previous app is moved to a hidden recoverable backup; nothing is deleted.
#
# Usage: ./scripts/install-desktop-macos.sh [target-triple]
# Run after `just desktop-release-build` (or `just desktop-release-build <target>`).

TARGET="${1:-aarch64-apple-darwin}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$REPO_ROOT/scripts/macos-signing-config.sh"
[[ -n "${BUZZ_MACOS_EXPECTED_TEAM_ID:-}" && "${BUZZ_MACOS_EXPECTED_AUTHORITY:-}" == "Developer ID Application:"* ]] || {
    echo "Error: canonical installation requires a configured Developer ID authority and Team ID." >&2
    exit 1
}
export BUZZ_MACOS_EXPECTED_TEAM_ID BUZZ_MACOS_EXPECTED_AUTHORITY
export BUZZ_MACOS_REQUIRE_TIMESTAMP=1 BUZZ_MACOS_REQUIRE_EMBEDDED_CHAIN=1
TARGET_DIR="$(cd "$REPO_ROOT" && cargo metadata --manifest-path desktop/src-tauri/Cargo.toml --format-version 1 --no-deps | node -p "JSON.parse(require('fs').readFileSync(0, 'utf8')).target_directory")"
BUILT_APP="$TARGET_DIR/$TARGET/release/bundle/macos/Buzz.app"
DEST="/Applications/Buzz.app"

if [[ ! -d "$BUILT_APP" ]]; then
    echo "Error: no built app at $BUILT_APP — run 'just desktop-release-build' first." >&2
    exit 1
fi

# Fail-closed run detection: `pgrep -x buzz-desktop` returns false negatives on
# some hosts (measured 2026-08-10: it missed a live buzz-desktop pid that ps and
# lsof both saw, and two rollout watchers tore the bundle mid-ditto as a result).
# Any single instrument reporting "running" blocks the install. The ps probe
# covers helper processes (buzz-acp et al) whose open handles tear a concurrent
# copy even after the main app exits.
RUNNING_EVIDENCE=""
if [[ -n "$(ps -axo comm= | awk -v p="$DEST/" 'index($0,p)==1')" ]]; then
    RUNNING_EVIDENCE="ps"
elif [[ -d "$DEST" ]] && lsof +D "$DEST" >/dev/null 2>&1; then
    RUNNING_EVIDENCE="lsof"
elif pgrep -x buzz-desktop >/dev/null 2>&1; then
    RUNNING_EVIDENCE="pgrep"
fi
if [[ -n "$RUNNING_EVIDENCE" ]]; then
    echo "Error: Buzz Desktop is running (detected via: $RUNNING_EVIDENCE). Quit it normally, then rerun this installer." >&2
    exit 2
fi

"$REPO_ROOT/scripts/verify-desktop-bundle.sh" "$TARGET"

# Preserve TCC identity across signed upgrades. The initial ad hoc-to-Developer
# ID migration has no stable old requirement and can need one final grant.
if [[ -d "$DEST" ]]; then
    old_metadata="$(codesign --display --verbose=2 "$DEST" 2>&1)"
    if grep -q '^Authority=Developer ID Application:' <<<"$old_metadata"; then
        old_requirement="$(codesign -d -r- "$DEST" 2>&1 | sed -n 's/^designated => //p')"
        new_requirement="$(codesign -d -r- "$BUILT_APP" 2>&1 | sed -n 's/^designated => //p')"
        [[ -n "$old_requirement" && -n "$new_requirement" ]] || {
            echo "Error: cannot establish signing identity continuity." >&2
            exit 1
        }
        codesign --verify -R="$old_requirement" "$BUILT_APP"
        codesign --verify -R="$new_requirement" "$DEST"
    else
        echo "Migrating to Developer ID signing; macOS may require a final permission grant."
    fi
fi

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
