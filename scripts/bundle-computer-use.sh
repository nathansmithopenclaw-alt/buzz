#!/usr/bin/env bash
# Stage a self-contained service; generated output is never used as source evidence.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
STAGE=$(mktemp -d "${TMPDIR:-/tmp}/buzz-computer-bundle.XXXXXX")
trap 'rm -rf "$STAGE"' EXIT
pnpm --filter @buzz/computer-use deploy --prod --legacy "$STAGE/runtime"
# pnpm 11 legacy deploy changes the source workspace's dependency-state marker.
# Restore the development workspace explicitly before invoking its build tools.
pnpm install --ignore-scripts
# Legacy deploy adds a self-reference to the source package. It is unused by this
# service and must not escape the bundle or introduce a recursive resource link.
SELF_LINK="$STAGE/runtime/node_modules/.pnpm/node_modules/@buzz/computer-use"
if [[ -L "$SELF_LINK" ]]; then rm "$SELF_LINK"; fi
node --input-type=module - "$STAGE/runtime" <<'NODE'
import { readdir, realpath } from 'node:fs/promises';
import { join, relative } from 'node:path';
const root = await realpath(process.argv[2]);
async function check(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isSymbolicLink()) {
      const target = await realpath(path);
      if (relative(root, target).startsWith('..')) throw new Error(`Bundle link escapes runtime: ${path}`);
    } else if (entry.isDirectory()) await check(path);
  }
}
await check(root);
NODE
PLAYWRIGHT_BROWSERS_PATH="$STAGE/runtime/browsers" node "$STAGE/runtime/node_modules/playwright/cli.js" install chromium
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
mkdir -p .cache/computer-use
rm -rf .cache/computer-use/runtime
mv "$STAGE/runtime" .cache/computer-use/runtime
printf '%s\n' 'Computer-use runtime staged with isolated Chromium and pinned native backend.'
