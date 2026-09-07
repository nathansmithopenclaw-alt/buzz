#!/usr/bin/env node
import { mkdir, writeFile, unlink, chmod } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { randomUUID } from "node:crypto";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { BrowserService } from "./browser.mjs";
import { computerServer } from "./mcp.mjs";
import { NativeService } from "./native.mjs";
import { Profiles } from "./profiles.mjs";
import { HostSlots } from "./slots.mjs";
import { startViewer } from "./viewer.mjs";

// Browser children receive a minimal environment, never inherited provider/relay credentials.
const bundledBrowsers = fileURLToPath(new URL("../browsers", import.meta.url));
if (existsSync(bundledBrowsers))
  process.env.PLAYWRIGHT_BROWSERS_PATH = bundledBrowsers;
const registry = process.env.BUZZ_COMPUTER_REGISTRY;
const nativeDriver = process.env.BUZZ_NATIVE_DRIVER;
const nativeSocket = process.env.BUZZ_NATIVE_SOCKET;
const owner = process.env.BUZZ_COMPUTER_OWNER || "local";
const label = process.env.BUZZ_COMPUTER_LABEL || "";
if (!registry)
  throw new Error(
    "BUZZ_COMPUTER_REGISTRY must name the host's private session registry",
  );
for (const key of Object.keys(process.env)) {
  if (
    !new Set([
      "PATH",
      "HOME",
      "TMPDIR",
      "TEMP",
      "TMP",
      "SystemRoot",
      "DISPLAY",
      "WAYLAND_DISPLAY",
      "XDG_RUNTIME_DIR",
      "LANG",
      "LC_ALL",
      "PLAYWRIGHT_BROWSERS_PATH",
    ]).has(key)
  )
    delete process.env[key];
}
const directory = resolve(registry);
await mkdir(directory, { recursive: true, mode: 0o700 });
await chmod(directory, 0o700);
const browser = new BrowserService({
  slots: new HostSlots(join(directory, "slots")),
  profiles: new Profiles(directory, owner),
});
const viewer = await startViewer(browser);
const native = new NativeService({
  driver: nativeDriver,
  socket: nativeSocket,
  registry: directory,
});
const server = computerServer(browser, native);
const id = randomUUID();
const receipt = join(directory, `${id}.json`);
try {
  await writeFile(
    receipt,
    JSON.stringify({
      version: 1,
      id,
      owner,
      label,
      pid: process.pid,
      created: new Date().toISOString(),
      viewerUrl: viewer.url,
    }),
    { mode: 0o600, flag: "wx" },
  );
} catch (error) {
  await viewer.close();
  throw error;
}
let closing;
const cleanup = () => {
  closing ??= (async () => {
    clearInterval(reaper);
    const results = await Promise.allSettled([
      browser.shutdown(),
      native.shutdown(),
      viewer.close(),
      unlink(receipt),
    ]);
    for (const result of results) {
      if (result.status === "rejected") {
        console.error("Computer-use cleanup failed:", result.reason);
        process.exitCode = 1;
      }
    }
  })();
  return closing;
};
const reaper = setInterval(() => {
  void Promise.all([browser.reap(), native.reap()]).catch((error) =>
    console.error(error.message),
  );
}, 30_000);
reaper.unref();
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"])
  process.once(signal, () => {
    void cleanup().then(() => process.exit(process.exitCode || 0));
  });
process.stdin.once("end", () => {
  void cleanup().then(() => process.exit(process.exitCode || 0));
});
try {
  await server.connect(new StdioServerTransport());
} catch (error) {
  await cleanup();
  throw error;
}
