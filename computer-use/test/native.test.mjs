import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { NativeService } from "../src/native.mjs";

const driver = process.env.BUZZ_TEST_NATIVE_DRIVER;
test("native observations bound default AX walks without overriding explicit limits", async (t) => {
  const directory = await mkdtemp("/tmp/buzz-native-bounds-");
  const native = new NativeService({
    driver: "test",
    socket: "test",
    registry: directory,
  });
  t.after(async () => {
    await native.shutdown();
    await rm(directory, { recursive: true });
  });
  const calls = [];
  native.connect = async () => ({
    callTool: async (request) => {
      calls.push(request);
      return { content: [] };
    },
  });
  await native.acquire();
  await native.call("native_get_window_state", { pid: 42, window_id: 7 });
  await native.call("native_get_accessibility_tree", {
    pid: 42,
    max_elements: 100,
    max_depth: 12,
    query: "Editor",
  });
  await native.call("native_list_windows", { pid: 42 });
  assert.deepEqual(calls, [
    {
      name: "get_window_state",
      arguments: { pid: 42, window_id: 7, max_elements: 30, max_depth: 5 },
    },
    {
      name: "get_accessibility_tree",
      arguments: { pid: 42, max_elements: 100, max_depth: 12, query: "Editor" },
    },
    { name: "list_windows", arguments: { pid: 42 } },
  ]);
});

test("content-only ACP clients receive native snapshot tokens alongside unchanged images", async (t) => {
  const directory = await mkdtemp("/tmp/buzz-native-content-");
  const native = new NativeService({
    driver: "test",
    socket: "test",
    registry: directory,
  });
  t.after(async () => {
    await native.shutdown();
    await rm(directory, { recursive: true });
  });
  const structuredContent = {
    snapshot_id: "snapshot-42",
    elements: [{ element_index: 1, element_token: "snapshot-42:1" }],
  };
  const image = { type: "image", mimeType: "image/png", data: "aW1hZ2U=" };
  const markdown = { type: "text", text: "[1] AXTextArea" };
  native.connect = async () => ({
    callTool: async () => ({
      structuredContent,
      content: [image, markdown],
    }),
  });
  await native.acquire();
  const result = await native.call("native_get_window_state", {});
  const rendered = result.content
    .filter((block) => block.type === "text")
    .map((block) => block.text);
  assert.deepEqual(JSON.parse(rendered[0]), structuredContent);
  assert.equal(result.structuredContent, structuredContent);
  assert.equal(result.content[1], image);
  assert.equal(result.content[2], markdown);
  // Clients that already got JSON text should not receive it twice.
  native.connect = async () => ({ callTool: async () => result });
  assert.equal(
    (await native.call("native_get_window_state", {})).content.length,
    3,
  );
});

test("pinned native daemon discovers real tools, arbitrates proxies and exits on parent EOF", {
  skip: !driver,
}, async (t) => {
  const directory = await mkdtemp("/tmp/buzz-native-test-");
  const socket = join(directory, "driver.sock");
  const daemon = spawn(
    driver,
    [
      "serve",
      "--embedded",
      "--parent-liveness-stdio",
      "--no-permissions-gate",
      "--socket",
      socket,
      "--host-bundle-id",
      "xyz.block.buzz.app",
      "--permission-mode",
      "standard",
    ],
    {
      env: {
        HOME: process.env.HOME,
        PATH: process.env.PATH,
        CUA_DRIVER_EMBEDDED: "1",
        CUA_DRIVER_EMBEDDED_HOST_PID: String(process.pid),
        CUA_DRIVER_RS_TELEMETRY_ENABLED: "false",
      },
      stdio: ["pipe", "ignore", "pipe"],
    },
  );
  let diagnostics = "";
  daemon.stderr.on("data", (bytes) => {
    diagnostics = (diagnostics + bytes).slice(-12000);
  });
  const exited = new Promise((resolve) =>
    daemon.once("exit", (code) => resolve(code)),
  );
  t.after(async () => {
    if (daemon.exitCode === null) daemon.kill("SIGKILL");
    await exited;
    await rm(directory, { recursive: true, force: true });
  });
  for (
    let i = 0;
    i < 100 && !existsSync(socket) && daemon.exitCode === null;
    i++
  )
    await new Promise((r) => setTimeout(r, 50));
  assert.ok(existsSync(socket), diagnostics);
  const a = new NativeService({ driver, socket, registry: directory });
  const b = new NativeService({ driver, socket, registry: directory });
  t.after(async () => {
    await a.shutdown();
    await b.shutdown();
  });
  const tools = await a.listTools();
  assert.ok(tools.some((tool) => tool.name === "native_get_window_state"));
  assert.ok(
    !tools.some(
      (tool) =>
        tool.name.includes("kill_app") || tool.name.includes("set_config"),
    ),
  );
  await a.acquire();
  await assert.rejects(b.acquire(), /capacity/);
  const permissions = await a.call("native_check_permissions", {
    prompt: false,
  });
  assert.notEqual(permissions.isError, true, JSON.stringify(permissions));
  assert.equal(permissions.content[0].type, "text");
  await a.relinquish();
  await b.acquire();
  const health = await b.call("native_health_report", {
    include: ["binary_version"],
  });
  assert.match(JSON.stringify(health), /0\.23\.2/);
  await b.relinquish();
  await Promise.all([a.shutdown(), b.shutdown()]);
  daemon.stdin.end();
  const outcome = await Promise.race([
    exited,
    new Promise((r) => setTimeout(() => r("timeout"), 5000)),
  ]);
  assert.notEqual(outcome, "timeout", diagnostics);
});

test("native input requires fresh observation and idle leases yield without stealing active work", async (t) => {
  const directory = await mkdtemp("/tmp/buzz-native-lease-");
  const native = new NativeService({
    driver: "test",
    socket: "test",
    registry: directory,
  });
  t.after(async () => {
    await native.shutdown();
    await rm(directory, { recursive: true });
  });
  let calls = 0;
  native.connect = async () => ({
    callTool: async () => {
      calls++;
      return { content: [{ type: "text", text: "fixture" }] };
    },
  });
  await native.acquire();
  await assert.rejects(
    native.call("native_press_key", { key: "a" }),
    /Observe the target/,
  );
  assert.equal(calls, 0);
  await native.call("native_get_window_state", {});
  await native.call("native_press_key", { key: "a" });
  await assert.rejects(
    native.call("native_press_key", { key: "b" }),
    /Observe the target/,
  );
  native.lastUsed = Date.now() - 61_000;
  native.active = true;
  await native.reap();
  assert.ok(native.release);
  native.active = false;
  await native.reap();
  assert.equal(native.release, null);
  await assert.rejects(
    native.call("native_get_window_state", {}),
    /native_acquire/,
  );
});
