#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:-aarch64-apple-darwin}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="$(cd "$REPO_ROOT" && cargo metadata --manifest-path desktop/src-tauri/Cargo.toml --format-version 1 --no-deps | node -p "JSON.parse(require('fs').readFileSync(0, 'utf8')).target_directory")"
APP="${2:-$TARGET_DIR/$TARGET/release/bundle/macos/Buzz.app}"
INFO="$APP/Contents/Info.plist"
MACOS="$APP/Contents/MacOS"

[[ -d "$APP" ]] || { echo "Error: app bundle not found: $APP" >&2; exit 1; }
[[ -f "$INFO" ]] || { echo "Error: Info.plist missing: $INFO" >&2; exit 1; }

identifier="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$INFO")"
[[ "$identifier" == "xyz.block.buzz.app" ]] || {
    echo "Error: unexpected bundle identifier: $identifier" >&2
    exit 1
}

for bin in buzz-desktop buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr buzz; do
    [[ -x "$MACOS/$bin" ]] || {
        echo "Error: bundled executable missing or not executable: $MACOS/$bin" >&2
        exit 1
    }
done

codesign --verify --deep --strict "$APP"

desktop_hash="$(shasum -a 256 "$MACOS/buzz-desktop" | awk '{print $1}')"
broker_hash="$(shasum -a 256 "$MACOS/buzz-acp" | awk '{print $1}')"
printf 'verified_bundle=%s\nidentifier=%s\ndesktop_sha256=%s\nbroker_sha256=%s\n' \
    "$APP" "$identifier" "$desktop_hash" "$broker_hash"
