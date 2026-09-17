import { assertDetachedBrowser } from "../../scripts/require-detached-browser.mjs";
assertDetachedBrowser();

import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import {
  mkdtemp,
  realpath,
  mkdir,
  writeFile,
  readFile,
  readdir,
  rm,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { randomUUID } from "node:crypto";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";

test("real MCP process cancellation preserves a pending Chromium POST and fences replacement input", {
  timeout: 30000,
}, async (t) => {
  const root = await realpath(
    await mkdtemp(join(tmpdir(), "buzz-evidence-stdio-")),
  );
  t.after(() => rm(root, { recursive: true, force: true }));
  let submitted;
  const receipt = new Promise((resolve) => {
    submitted = resolve;
  });
  let count = 0;
  const web = createServer((req, res) => {
    if (req.method === "POST") {
      count++;
      submitted();
      return;
    }
    res.setHeader("Content-Type", "text/html");
    res.end('<form method="POST"><button>Submit once</button></form>');
  });
  await new Promise((resolve) => web.listen(0, "127.0.0.1", resolve));
  t.after(
    () =>
      new Promise((resolve) => {
        web.closeAllConnections();
        web.close(resolve);
      }),
  );
  const url = `http://127.0.0.1:${web.address().port}`;
  const execution = join(root, "execution");
  await mkdir(execution);
  async function grant(recovery) {
    const session = randomUUID();
    const turn_id = randomUUID();
    const calls = join(execution, "turns", turn_id, "calls");
    await mkdir(calls, { recursive: true });
    await writeFile(join(execution, "session.json"), JSON.stringify(session));
    await writeFile(
      join(execution, "active.json"),
      JSON.stringify({ version: 1, session, turn_id, active: true, recovery }),
    );
    const client = new Client({ name: "execution-conformance", version: "1" });
    const transport = new StdioClientTransport({
      command: process.execPath,
      args: [fileURLToPath(new URL("../src/main.mjs", import.meta.url))],
      env: {
        PATH: process.env.PATH,
        HOME: process.env.HOME,
        BUZZ_COMPUTER_REGISTRY: root,
        BUZZ_EXECUTION_ROOT: execution,
        BUZZ_EXECUTION_SESSION: session,
      },
      stderr: "inherit",
    });
    await client.connect(transport);
    t.after(() => client.close());
    return { client, calls };
  }
  const first = await grant(false);
  const opened = await first.client.callTool({
    name: "browser_open",
    arguments: { url },
  });
  assert.notEqual(opened.isError, true);
  const session = JSON.parse(opened.content[0].text).id;
  const observation = await first.client.callTool({
    name: "browser_observe",
    arguments: { session },
  });
  assert.equal(observation.content[1].type, "image");
  const view = JSON.parse(observation.content[0].text);
  const pending = first.client
    .callTool({
      name: "browser_action",
      arguments: {
        session,
        kind: "click",
        ref: view.elements.find((e) => e.tag === "button").ref,
        generation: view.generation,
        observation: view.observation,
      },
    })
    .catch((error) => error);
  await receipt;
  const records = await Promise.all(
    (await readdir(first.calls))
      .filter((n) => n.endsWith(".json"))
      .map(async (n) =>
        JSON.parse(await readFile(join(first.calls, n), "utf8")),
      ),
  );
  assert.equal(
    records.find((r) => r.tool === "browser_action").state,
    "started",
  );
  await first.client.close();
  await pending;
  const next = await grant(true);
  const list = await next.client.callTool({
    name: "browser_list",
    arguments: {},
  });
  assert.notEqual(list.isError, true);
  const retry = await next.client.callTool({
    name: "browser_open",
    arguments: { url },
  });
  assert.equal(retry.isError, true);
  assert.match(retry.content[0].text, /mode fences/);
  assert.equal(count, 1);
  await next.client.close();
  assert.equal(
    (await readdir(first.calls)).filter((n) => n.endsWith(".json")).length,
    3,
  );
});
