#!/usr/bin/env bash
set -euo pipefail

# Sign a locally built Buzz.app. Developer ID signing is deliberately
# fail-closed: the caller must select both the keychain and expected Team ID so
# a similarly named or duplicated identity cannot silently sign a rollout.
#
# Usage: ./scripts/sign-desktop-macos.sh [target-triple] [path-to-Buzz.app]
#
# Ad hoc development signing (default):
#   ./scripts/sign-desktop-macos.sh
#
# Developer ID rollout signing:
#   BUZZ_MACOS_SIGNING_IDENTITY='<certificate SHA-1 or exact common name>' \
#   BUZZ_MACOS_SIGNING_KEYCHAIN="$HOME/Library/Keychains/login.keychain-db" \
#   BUZZ_MACOS_EXPECTED_TEAM_ID='<10-character Team ID>' \
#   ./scripts/sign-desktop-macos.sh

TARGET="${1:-aarch64-apple-darwin}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="$(cd "$REPO_ROOT" && cargo metadata --manifest-path desktop/src-tauri/Cargo.toml --format-version 1 --no-deps | node -p "JSON.parse(require('fs').readFileSync(0, 'utf8')).target_directory")"
APP="${2:-$TARGET_DIR/$TARGET/release/bundle/macos/Buzz.app}"
ENTITLEMENTS="$REPO_ROOT/desktop/src-tauri/Entitlements.plist"
IDENTITY="${BUZZ_MACOS_SIGNING_IDENTITY:--}"
KEYCHAIN="${BUZZ_MACOS_SIGNING_KEYCHAIN:-}"
EXPECTED_TEAM_ID="${BUZZ_MACOS_EXPECTED_TEAM_ID:-}"
VERIFY_DELAY="${BUZZ_MACOS_SIGNING_VERIFY_DELAY_SECONDS:-8}"

die() {
    echo "Error: $*" >&2
    exit 1
}

[[ "$(uname -s)" == "Darwin" ]] || die "macOS code signing is only available on Darwin"
[[ -d "$APP" ]] || die "app bundle not found: $APP"
[[ -f "$ENTITLEMENTS" ]] || die "entitlements file not found: $ENTITLEMENTS"
[[ "$VERIFY_DELAY" =~ ^[0-9]+$ ]] || die "BUZZ_MACOS_SIGNING_VERIFY_DELAY_SECONDS must be an integer from 0 to 60"
(( VERIFY_DELAY <= 60 )) || die "BUZZ_MACOS_SIGNING_VERIFY_DELAY_SECONDS must be an integer from 0 to 60"

SIGN_ARGS=(--force --deep --sign "$IDENTITY")
EXPECTED_AUTHORITY=""
if [[ "$IDENTITY" != "-" ]]; then
    [[ -n "$KEYCHAIN" ]] || die "BUZZ_MACOS_SIGNING_KEYCHAIN is required for non-ad-hoc signing"
    [[ -f "$KEYCHAIN" ]] || die "signing keychain not found: $KEYCHAIN"
    [[ "$EXPECTED_TEAM_ID" =~ ^[A-Z0-9]{10}$ ]] || die "BUZZ_MACOS_EXPECTED_TEAM_ID must be a 10-character Team ID for non-ad-hoc signing"

    IDENTITIES="$(security find-identity -v -p codesigning "$KEYCHAIN")"
    if [[ "$IDENTITY" =~ ^[[:xdigit:]]{40}$ ]]; then
        IDENTITY_SHA="$(printf '%s' "$IDENTITY" | tr '[:lower:]' '[:upper:]')"
        IDENTITY_LINE="$(printf '%s\n' "$IDENTITIES" | awk -v sha="$IDENTITY_SHA" 'toupper($2) == sha { print; exit }')"
    else
        IDENTITY_LINE="$(printf '%s\n' "$IDENTITIES" | grep -F "\"$IDENTITY\"" | head -1 || true)"
    fi
    [[ -n "$IDENTITY_LINE" ]] || die "signing identity is not valid in keychain $KEYCHAIN: $IDENTITY"
    EXPECTED_AUTHORITY="$(printf '%s\n' "$IDENTITY_LINE" | sed -n 's/.*"\(.*\)".*/\1/p')"
    [[ -n "$EXPECTED_AUTHORITY" ]] || die "could not resolve the signing identity common name"

    SIGN_ARGS=(--force --deep --options runtime --timestamp --keychain "$KEYCHAIN" --sign "$IDENTITY")
fi

codesign "${SIGN_ARGS[@]}" --entitlements "$ENTITLEMENTS" "$APP"

if [[ "$IDENTITY" == "-" ]]; then
    BUZZ_MACOS_SIGNING_VERIFY_DELAY_SECONDS=0 \
        "$REPO_ROOT/scripts/verify-desktop-bundle.sh" "$TARGET" "$APP"
else
    BUZZ_MACOS_EXPECTED_AUTHORITY="$EXPECTED_AUTHORITY" \
    BUZZ_MACOS_EXPECTED_TEAM_ID="$EXPECTED_TEAM_ID" \
    BUZZ_MACOS_REQUIRE_TIMESTAMP=1 \
    BUZZ_MACOS_REQUIRE_EMBEDDED_CHAIN=1 \
    BUZZ_MACOS_SIGNING_VERIFY_DELAY_SECONDS="$VERIFY_DELAY" \
        "$REPO_ROOT/scripts/verify-desktop-bundle.sh" "$TARGET" "$APP"
fi
