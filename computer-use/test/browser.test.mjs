import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { BrowserService } from "../src/browser.mjs";
import { startViewer } from "../src/viewer.mjs";

const mainScript =
  process.env.BUZZ_TEST_COMPUTER_SCRIPT ||
  fileURLToPath(new URL("../src/main.mjs", import.meta.url));

const markup = `<!doctype html><title>Browser isolation test</title><label>Name <input id=name></label><button id=save>Save</button><output id=result></output><script>save.onclick=()=>{localStorage.setItem('name',document.getElementById('name').value);document.cookie='name='+document.getElementById('name').value;result.textContent=document.getElementById('name').value}</script>`;
async function fixture(t) {
  const server = createServer((_req, res) => {
    res.setHeader("Content-Type", "text/html");
    res.end(markup);
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(
    () =>
      new Promise((resolve) => {
        server.closeAllConnections();
        server.close(resolve);
      }),
  );
  return `http://127.0.0.1:${server.address().port}`;
}
async function fill(browser, id, value, actor = "agent") {
  const observation = await browser.observe(id);
  const ref = observation.elements.find((e) => e.tag === "input").ref;
  return browser.act(
    id,
    {
      kind: "fill",
      ref,
      text: value,
      generation: observation.generation,
      observation: observation.observation,
    },
    actor,
  );
}

test("real Chromium isolates storage and input, fences stale refs, and closes contexts", async (t) => {
  const url = await fixture(t);
  const browser = new BrowserService({ maxSessions: 2 });
  t.after(() => browser.shutdown());
  const [a, b] = await Promise.all([browser.open(url), browser.open(url)]);
  await assert.rejects(browser.open(url), /session limit/);
  await Promise.all([
    fill(browser, a.id, "alpha"),
    fill(browser, b.id, "beta"),
  ]);
  for (const s of [a, b]) {
    const o = await browser.observe(s.id);
    assert.equal(o.image[0], 0xff);
    const save = o.elements.find((e) => e.tag === "button").ref;
    await browser.act(s.id, {
      kind: "click",
      ref: save,
      generation: o.generation,
      observation: o.observation,
    });
    await assert.rejects(
      browser.act(s.id, {
        kind: "click",
        ref: save,
        generation: o.generation,
        observation: o.observation,
      }),
      /Stale element/,
    );
  }
  assert.equal(
    await browser.get(a.id).page.evaluate(() => localStorage.name),
    "alpha",
  );
  assert.equal(
    await browser.get(b.id).page.evaluate(() => localStorage.name),
    "beta",
  );
  assert.equal((await browser.get(a.id).context.cookies())[0].value, "alpha");
  assert.equal((await browser.get(b.id).context.cookies())[0].value, "beta");
  const stale = await browser.observe(a.id);
  await browser.control(a.id, "human");
  await assert.rejects(
    fill(browser, a.id, "agent-after-takeover"),
    /owned by human/,
  );
  await fill(browser, a.id, "human", "human");
  await browser.control(a.id, "agent");
  await assert.rejects(
    browser.act(a.id, {
      kind: "type",
      text: "stale",
      generation: stale.generation,
    }),
    /Stale page/,
  );
  await fill(browser, a.id, "resumed");
  const context = browser.get(a.id).context;
  await browser.close(a.id);
  assert.equal(context.pages().length, 0);
  assert.throws(() => browser.get(a.id), /closed/);
  await browser.close(b.id);
  assert.equal(browser.list().length, 0);
  assert.equal(browser.browser, null);
});

test("absolute lifetime expires even if worker activity keeps resetting idle time", async (t) => {
  let now = 0;
  const browser = new BrowserService({
    idleMs: 100,
    lifetimeMs: 200,
    now: () => now,
  });
  t.after(() => browser.shutdown());
  const a = await browser.open();
  now = 90;
  await browser.observe(a.id);
  now = 180;
  await browser.observe(a.id);
  now = 201;
  await browser.reap();
  assert.equal(browser.list().length, 0);
});

test("private viewer requires token and same origin; human action changes the agent's exact page", async (t) => {
  const url = await fixture(t);
  const browser = new BrowserService();
  const viewer = await startViewer(browser);
  t.after(async () => {
    await viewer.close();
    await browser.shutdown();
  });
  const a = await browser.open(url);
  const address = new URL(viewer.url);
  const headers = {
    Authorization: `Bearer ${address.hash.slice(1)}`,
    "Content-Type": "application/json",
  };
  const call = async (body, override = {}) =>
    fetch(`${address.origin}/control`, {
      method: "POST",
      headers: { ...headers, ...override },
      body: JSON.stringify(body),
    });
  assert.equal(
    (await call({ op: "list" }, { Authorization: "wrong" })).status,
    401,
  );
  assert.equal(
    (await call({ op: "list" }, { Origin: "https://hostile.example" })).status,
    403,
  );
  await call({ op: "control", id: a.id, controller: "human" });
  const o = await browser.observe(a.id);
  const response = await call({
    op: "action",
    id: a.id,
    action: {
      kind: "fill",
      text: "same live page",
      ref: 0,
      generation: o.generation,
      observation: o.observation,
    },
  });
  assert.equal(response.status, 200);
  assert.equal(
    (await browser.observe(a.id)).elements[0].value,
    "same live page",
  );
  assert.ok(
    (await (await call({ op: "frame", id: a.id })).json()).image.length > 100,
  );
});

test("standard MCP stdio discovers tools, returns real image blocks, and cleans receipt on EOF", async (t) => {
  const registry = await mkdtemp(join(tmpdir(), "buzz-browser-test-"));
  t.after(() => rm(registry, { recursive: true, force: true }));
  const client = new Client({ name: "conformance", version: "1" });
  const transport = new StdioClientTransport({
    command: process.env.BUZZ_TEST_COMPUTER_CLI || process.execPath,
    args: process.env.BUZZ_TEST_COMPUTER_CLI
      ? ["computer", "mcp"]
      : [mainScript],
    env: {
      PATH: process.env.PATH,
      HOME: process.env.HOME,
      BUZZ_COMPUTER_NODE: process.execPath,
      BUZZ_COMPUTER_SCRIPT: mainScript,
      BUZZ_COMPUTER_REGISTRY: registry,
      BUZZ_COMPUTER_OWNER: "test-agent",
    },
    stderr: "pipe",
  });
  t.after(() => client.close());
  await client.connect(transport);
  const listed = await client.listTools();
  assert.ok(listed.tools.some((tool) => tool.name === "browser_observe"));
  const opened = await client.callTool({ name: "browser_open", arguments: {} });
  const session = JSON.parse(opened.content[0].text).id;
  const observed = await client.callTool({
    name: "browser_observe",
    arguments: { session },
  });
  assert.equal(observed.isError, undefined);
  assert.equal(observed.content[1].type, "image");
  assert.equal(observed.content[1].mimeType, "image/jpeg");
  assert.equal(Buffer.from(observed.content[1].data, "base64")[0], 0xff);
  const entries = (await readdir(registry)).filter((name) =>
    name.endsWith(".json"),
  );
  const receipt = JSON.parse(
    await readFile(join(registry, entries[0]), "utf8"),
  );
  assert.equal(receipt.owner, "test-agent");
  assert.ok(
    !JSON.stringify(observed).includes(
      new URL(receipt.viewerUrl).hash.slice(1),
    ),
  );
  await client.close();
  for (
    let i = 0;
    i < 50 && (await readdir(registry)).some((name) => name.endsWith(".json"));
    i++
  )
    await new Promise((resolve) => setTimeout(resolve, 100));
  assert.deepEqual(
    (await readdir(registry)).filter((name) => name.endsWith(".json")),
    [],
  );
});

test("killing an owned MCP process leaves no Chromium descendants running", async (t) => {
  const { execFile } = await import("node:child_process");
  const { promisify } = await import("node:util");
  const exec = promisify(execFile);
  const registry = await mkdtemp(join(tmpdir(), "buzz-browser-crash-"));
  t.after(() => rm(registry, { recursive: true, force: true }));
  const client = new Client({ name: "crash-conformance", version: "1" });
  const transport = new StdioClientTransport({
    command: process.env.BUZZ_TEST_COMPUTER_CLI || process.execPath,
    args: process.env.BUZZ_TEST_COMPUTER_CLI
      ? ["computer", "mcp"]
      : [mainScript],
    env: {
      PATH: process.env.PATH,
      HOME: process.env.HOME,
      BUZZ_COMPUTER_NODE: process.execPath,
      BUZZ_COMPUTER_SCRIPT: mainScript,
      BUZZ_COMPUTER_REGISTRY: registry,
      BUZZ_COMPUTER_OWNER: "crash-test",
    },
    stderr: "pipe",
  });
  t.after(() => client.close());
  await client.connect(transport);
  const result = await client.callTool({ name: "browser_open", arguments: {} });
  assert.notEqual(result.isError, true, JSON.stringify(result));
  const receiptPath = (await readdir(registry)).find((name) =>
    name.endsWith(".json"),
  );
  const { pid } = JSON.parse(
    await readFile(join(registry, receiptPath), "utf8"),
  );
  const { stdout } = await exec("ps", ["-axo", "pid=,ppid="]);
  const pairs = stdout
    .trim()
    .split("\n")
    .map((line) => line.trim().split(/\s+/).map(Number));
  const descendants = new Set([pid]);
  for (let i = 0; i < 8; i++)
    for (const [child, parent] of pairs)
      if (descendants.has(parent)) descendants.add(child);
  assert.ok(
    descendants.size >= 3,
    "test must own actual Chromium subprocesses",
  );
  process.kill(pid, "SIGKILL");
  const live = () =>
    [...descendants].filter((id) => {
      try {
        process.kill(id, 0);
        return true;
      } catch (error) {
        if (error.code === "ESRCH") return false;
        throw error;
      }
    });
  for (let i = 0; i < 100 && live().length; i++)
    await new Promise((r) => setTimeout(r, 50));
  assert.deepEqual(live(), []);
});
