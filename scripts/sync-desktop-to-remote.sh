#!/usr/bin/env bash
set -euo pipefail

# Copies the local canonical /Applications/Buzz.app to a remote macOS host
# (matching CPU architecture required) and installs it there the same way
# install-desktop-macos.sh does locally: quit any running instance, remove
# existing Buzz*.app copies, replace with the freshly copied bundle.
#
# Usage: ./scripts/sync-desktop-to-remote.sh <ssh-host-alias>
# Requires: passwordless ssh to <ssh-host-alias>, and a local
# /Applications/Buzz.app (run ./scripts/install-desktop-macos.sh first).
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

echo "Copying to $REMOTE_HOST..."
scp "$TMP_TAR" "$REMOTE_HOST:/tmp/buzz-desktop-sync.tar"
rm -f "$TMP_TAR"

echo "Installing on $REMOTE_HOST (quitting running instance, replacing canonical copy)..."
ssh "$REMOTE_HOST" bash -s <<'REMOTE_SCRIPT'
set -euo pipefail
pkill -x buzz-desktop 2>/dev/null || true
sleep 1
find /Applications -maxdepth 1 -iname "Buzz*.app" -exec rm -rf {} +
cd /Applications
tar -xf /tmp/buzz-desktop-sync.tar
rm -f /tmp/buzz-desktop-sync.tar
echo "Installed on $(hostname)."
REMOTE_SCRIPT

echo "Done. Relaunch Buzz on $REMOTE_HOST when ready."
