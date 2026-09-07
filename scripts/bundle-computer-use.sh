#!/usr/bin/env bash
# Stage a self-contained service; generated output is never used as source evidence.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
STAGE=$(mktemp -d "${TMPDIR:-/tmp}/buzz-computer-bundle.XXXXXX")
trap 'rm -rf "$STAGE"' EXIT
# Tauri's resource copier omits symlinks. Modern deploy preserves the workspace
# lockfile while the hoisted linker produces real package directories. The
# injected-workspace option is local to this command, not a workspace mutation.
pnpm --filter @buzz/computer-use --config.node-linker=hoisted \
  --config.injectWorkspacePackages=true deploy --prod "$STAGE/runtime"
# Runtime uses explicit JS entry points, never package-manager shims or patches.
rm -rf "$STAGE/runtime/node_modules/.bin"
rm -f "$STAGE/runtime/pnpm-workspace.yaml"

# Headless shell is the controlled engine; shipping the unused full Chrome app
# also introduces framework symlinks that Tauri cannot preserve.
PLAYWRIGHT_BROWSERS_PATH="$STAGE/runtime/browsers" node "$STAGE/runtime/node_modules/playwright/cli.js" install --only-shell chromium
# macOS native backend is optional on other targets; this pinned package is prerelease.
if [[ "${1:-$(uname -m)}" == "aarch64-apple-darwin" || "${1:-$(uname -m)}" == "arm64" ]]; then
    ARCHIVE="$STAGE/cua-driver.tar.gz"
    curl --fail --location --retry 3 \
      https://github.com/trycua/cua/releases/download/cua-driver-rs-v0.23.2/cua-driver-rs-0.23.2-darwin-arm64.tar.gz \
      --output "$ARCHIVE"
    EXPECTED=c606a0410eb1bf59ee81d697f6fbf8b7126b2e9a3f802272a34807b45b6ecd6f
    ACTUAL=$(shasum -a 256 "$ARCHIVE" | cut -d ' ' -f 1)
    [[ "$ACTUAL" == "$EXPECTED" ]] || { echo "CuaDriver checksum mismatch" >&2; exit 1; }
    tar -xzf "$ARCHIVE" -C "$STAGE"
    mkdir -p "$STAGE/runtime/native"
    cp "$STAGE/cua-driver-rs-0.23.2-darwin-arm64/cua-driver" "$STAGE/runtime/native/"
    cp "$STAGE/cua-driver-rs-0.23.2-darwin-arm64/cua-cursor-theme" "$STAGE/runtime/native/"
    curl --fail --location https://raw.githubusercontent.com/trycua/cua/cua-driver-rs-v0.23.2/LICENSE.md --output "$STAGE/runtime/native/LICENSE.md"
fi
node --input-type=module - "$STAGE/runtime" <<'NODE'
import { readdir, realpath } from 'node:fs/promises';
import { join } from 'node:path';
const root = await realpath(process.argv[2]);
async function check(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isSymbolicLink()) {
      throw new Error(`Tauri would omit this runtime symlink: ${path}`);
    } else if (entry.isDirectory()) await check(path);
  }
}
await check(root);
NODE
mkdir -p .cache/computer-use
rm -rf .cache/computer-use/runtime
mv "$STAGE/runtime" .cache/computer-use/runtime
printf '%s\n' 'Computer-use runtime staged with isolated Chromium and pinned native backend.'
