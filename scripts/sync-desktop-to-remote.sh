#!/usr/bin/env bash
set -euo pipefail

# Copies the local canonical /Applications/Buzz.app to a remote macOS host
# (matching CPU architecture required) and installs it there the same way
# install-desktop-macos.sh does locally: quit any running instance, park
# existing Buzz*.app copies as a rollback backup, replace with the freshly
# copied bundle, and verify the installed payload by checksum.
#
# Usage: ./scripts/sync-desktop-to-remote.sh <ssh-host-alias>
# Requires: passwordless ssh to <ssh-host-alias>, and a local
# /Applications/Buzz.app (run ./scripts/install-desktop-macos.sh first).
#
# Existing copies are never deleted: they move to
# /Applications/.buzz-backups/<timestamp>/ for one-step rollback.
#
# Quitting Buzz on the remote host also stops any agent sidecar processes
# (buzz-acp, etc.) it was hosting — confirm that's acceptable before running
# this against a host with active agent sessions.

REMOTE_HOST="${1:?Usage: sync-desktop-to-remote.sh <ssh-host-alias>}"
LOCAL_APP="/Applications/Buzz.app"

if [[ ! -d "$LOCAL_APP" ]]; then
    echo "Error: no local $LOCAL_APP — run ./scripts/install-desktop-macos.sh first." >&2
    exit 1
fi

echo "Archiving local build..."
TMP_TAR="$(mktemp -t buzz-desktop-sync).tar"
(cd /Applications && tar -cf "$TMP_TAR" "Buzz.app")
LOCAL_SHA="$(shasum -a 256 "$TMP_TAR" | awk '{print $1}')"

echo "Copying to $REMOTE_HOST..."
scp "$TMP_TAR" "$REMOTE_HOST:/tmp/buzz-desktop-sync.tar"
rm -f "$TMP_TAR"

echo "Installing on $REMOTE_HOST (quitting running instance, parking old copies, verifying payload)..."
ssh "$REMOTE_HOST" bash -s "$LOCAL_SHA" <<'REMOTE_SCRIPT'
set -euo pipefail
EXPECTED_SHA="${1:?missing expected sha}"
REMOTE_SHA="$(shasum -a 256 /tmp/buzz-desktop-sync.tar | awk '{print $1}')"
if [[ "$REMOTE_SHA" != "$EXPECTED_SHA" ]]; then
    echo "Error: transferred archive sha mismatch (expected $EXPECTED_SHA, got $REMOTE_SHA) — aborting before touching /Applications." >&2
    exit 1
fi
pkill -x buzz-desktop 2>/dev/null || true
sleep 1
BACKUP_DIR="/Applications/.buzz-backups/$(date +%Y%m%d-%H%M%S)"
shopt -s nullglob nocaseglob
existing=(/Applications/Buzz*.app)
shopt -u nocaseglob
if (( ${#existing[@]} > 0 )); then
    mkdir -p "$BACKUP_DIR"
    for app in "${existing[@]}"; do
        mv "$app" "$BACKUP_DIR/"
    done
    echo "Parked ${#existing[@]} existing bundle(s) in $BACKUP_DIR."
fi
cd /Applications
tar -xf /tmp/buzz-desktop-sync.tar
rm -f /tmp/buzz-desktop-sync.tar
if [[ ! -d /Applications/Buzz.app ]]; then
    echo "Error: extraction completed but /Applications/Buzz.app is missing — previous bundles remain parked in $BACKUP_DIR." >&2
    exit 1
fi
echo "Installed on $(hostname); payload sha verified ($REMOTE_SHA)."
REMOTE_SCRIPT

echo "Done. Previous bundles (if any) parked under /Applications/.buzz-backups/ on $REMOTE_HOST — rollback = quit Buzz, move the parked bundle back. Relaunch Buzz on $REMOTE_HOST when ready."
