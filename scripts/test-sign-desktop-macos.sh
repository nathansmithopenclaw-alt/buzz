#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT="$(mktemp -d -t buzz-signing-test)"
trap 'rm -rf "$ROOT"' EXIT

pass_count=0
fail() {
    echo "FAIL: $*" >&2
    exit 1
}
pass() {
    pass_count=$((pass_count + 1))
    echo "PASS: $*"
}

APP="$ROOT/Buzz.app"
MACOS="$APP/Contents/MacOS"
mkdir -p "$MACOS" "$APP/Contents/Resources"
cp /bin/echo "$MACOS/buzz-desktop"
for bin in buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr buzz buzz-backend-kubernetes; do
    cp /bin/echo "$MACOS/$bin"
done

cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>buzz-desktop</string>
  <key>CFBundleIdentifier</key><string>xyz.block.buzz.app</string>
  <key>CFBundleName</key><string>Buzz</string>
  <key>CFBundlePackageType</key><string>APPL</string>
</dict>
</plist>
PLIST

BUZZ_MACOS_SIGNING_IDENTITY=- BUZZ_MACOS_SIGNING_VERIFY_DELAY_SECONDS=0 \
    "$REPO_ROOT/scripts/sign-desktop-macos.sh" aarch64-apple-darwin "$APP" >/dev/null
"$REPO_ROOT/scripts/verify-desktop-bundle.sh" aarch64-apple-darwin "$APP" >/dev/null
pass "ad-hoc signing preserves the required entitlements and deep-verifies"

if BUZZ_MACOS_EXPECTED_TEAM_ID=AAAAAAAAAA \
    "$REPO_ROOT/scripts/verify-desktop-bundle.sh" aarch64-apple-darwin "$APP" >/dev/null 2>&1; then
    fail "verification accepted an unexpected TeamIdentifier"
fi
pass "verification rejects an unexpected TeamIdentifier"

if BUZZ_MACOS_SIGNING_IDENTITY='Developer ID Application: Missing (AAAAAAAAAA)' \
    BUZZ_MACOS_EXPECTED_TEAM_ID=AAAAAAAAAA \
    "$REPO_ROOT/scripts/sign-desktop-macos.sh" aarch64-apple-darwin "$APP" >/dev/null 2>&1; then
    fail "non-ad-hoc signing accepted a missing explicit keychain"
fi
pass "Developer ID signing requires an explicit keychain"

if BUZZ_MACOS_SIGNING_IDENTITY=- BUZZ_MACOS_SIGNING_VERIFY_DELAY_SECONDS=61 \
    "$REPO_ROOT/scripts/sign-desktop-macos.sh" aarch64-apple-darwin "$APP" >/dev/null 2>&1; then
    fail "signing accepted an excessive delayed-verification interval"
fi
pass "delayed-verification interval is bounded"

printf '\nall %d desktop signing contract checks passed\n' "$pass_count"
