#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:-aarch64-apple-darwin}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="$(cd "$REPO_ROOT" && cargo metadata --manifest-path desktop/src-tauri/Cargo.toml --format-version 1 --no-deps | node -p "JSON.parse(require('fs').readFileSync(0, 'utf8')).target_directory")"
APP="${2:-$TARGET_DIR/$TARGET/release/bundle/macos/Buzz.app}"
INFO="$APP/Contents/Info.plist"
MACOS="$APP/Contents/MacOS"
EXPECTED_AUTHORITY="${BUZZ_MACOS_EXPECTED_AUTHORITY:-}"
EXPECTED_TEAM_ID="${BUZZ_MACOS_EXPECTED_TEAM_ID:-}"
REQUIRE_TIMESTAMP="${BUZZ_MACOS_REQUIRE_TIMESTAMP:-0}"
REQUIRE_EMBEDDED_CHAIN="${BUZZ_MACOS_REQUIRE_EMBEDDED_CHAIN:-0}"
VERIFY_DELAY="${BUZZ_MACOS_SIGNING_VERIFY_DELAY_SECONDS:-0}"

[[ -d "$APP" ]] || { echo "Error: app bundle not found: $APP" >&2; exit 1; }
[[ -f "$INFO" ]] || { echo "Error: Info.plist missing: $INFO" >&2; exit 1; }

identifier="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$INFO")"
[[ "$identifier" == "xyz.block.buzz.app" ]] || {
    echo "Error: unexpected bundle identifier: $identifier" >&2
    exit 1
}

BINS=(buzz-desktop buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr buzz)
if [[ "$TARGET" != *windows* ]]; then
    BINS+=(buzz-backend-kubernetes)
fi
for bin in "${BINS[@]}"; do
    [[ -x "$MACOS/$bin" ]] || {
        echo "Error: bundled executable missing or not executable: $MACOS/$bin" >&2
        exit 1
    }
done

[[ "$VERIFY_DELAY" =~ ^[0-9]+$ ]] && (( VERIFY_DELAY <= 60 )) || {
    echo "Error: BUZZ_MACOS_SIGNING_VERIFY_DELAY_SECONDS must be an integer from 0 to 60" >&2
    exit 1
}

codesign --verify --deep --strict --verbose=2 "$APP"
"$REPO_ROOT/desktop/scripts/verify-macos-entitlements.sh" "$APP"

if [[ "$REQUIRE_EMBEDDED_CHAIN" == "1" ]]; then
    cert_dir="$(mktemp -d -t buzz-codesign-certs)"
    trap 'rm -rf "$cert_dir"' EXIT
fi

verify_signature_contract() {
    local path="$1" label metadata prefix cert
    label="$(basename "$path")"
    metadata="$(codesign --display --verbose=4 "$path" 2>&1)"
    if [[ -n "$EXPECTED_AUTHORITY" ]] && ! grep -Fqx "Authority=$EXPECTED_AUTHORITY" <<<"$metadata"; then
        echo "Error: $label does not have expected authority: $EXPECTED_AUTHORITY" >&2
        exit 1
    fi
    if [[ -n "$EXPECTED_TEAM_ID" ]] && ! grep -Fqx "TeamIdentifier=$EXPECTED_TEAM_ID" <<<"$metadata"; then
        echo "Error: $label does not have expected TeamIdentifier: $EXPECTED_TEAM_ID" >&2
        exit 1
    fi
    if [[ "$REQUIRE_TIMESTAMP" == "1" ]] && ! grep -Eq '^Timestamp=' <<<"$metadata"; then
        echo "Error: $label does not contain a trusted timestamp" >&2
        exit 1
    fi
    if [[ "$REQUIRE_EMBEDDED_CHAIN" == "1" ]]; then
        prefix="$cert_dir/$label-cert"
        codesign --display --extract-certificates="$prefix" "$path" >/dev/null 2>&1
        for cert in 0 1 2; do
            [[ -s "$prefix$cert" ]] || {
                echo "Error: $label is missing embedded certificate chain member: cert$cert" >&2
                exit 1
            }
        done
        security verify-cert -p codeSign -c "${prefix}0" >/dev/null
    fi
}

if [[ -n "$EXPECTED_AUTHORITY" || -n "$EXPECTED_TEAM_ID" || "$REQUIRE_TIMESTAMP" == "1" || "$REQUIRE_EMBEDDED_CHAIN" == "1" ]]; then
    for bin in "${BINS[@]}"; do
        verify_signature_contract "$MACOS/$bin"
    done
fi

if (( VERIFY_DELAY > 0 )); then
    sleep "$VERIFY_DELAY"
    codesign --verify --deep --strict --verbose=2 "$APP"
fi

desktop_hash="$(shasum -a 256 "$MACOS/buzz-desktop" | awk '{print $1}')"
broker_hash="$(shasum -a 256 "$MACOS/buzz-acp" | awk '{print $1}')"
printf 'verified_bundle=%s\nidentifier=%s\ndesktop_sha256=%s\nbroker_sha256=%s\n' \
    "$APP" "$identifier" "$desktop_hash" "$broker_hash"
