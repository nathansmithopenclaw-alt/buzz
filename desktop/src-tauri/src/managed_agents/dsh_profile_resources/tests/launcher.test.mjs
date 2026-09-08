import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, cp, symlink, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { createServer } from "node:http";
const require = createRequire(import.meta.url);
const profile = fileURLToPath(new URL("../", import.meta.url));
const dsh = resolve(
  require.resolve("@deepseek-ai/dsh-acp"),
  "../../../dsh/lib/bin.js",
);
const liveEndpoint = process.env.DSH_TEST_ENDPOINT;
const testModel = process.env.DSH_TEST_MODEL ?? "fixture-model";
const nodeModules = resolve(dsh, "../../../../");

test("actual dsh profile boots, scopes MCP, calls tools and closes sibling sessions", {
  timeout: 30000,
}, async (t) => {
  const dir = await mkdtemp(join(tmpdir(), "buzz-dsh-test-"));
  let child,
    stderr = "",
    sequence = 0;
  const pending = new Map(),
    updates = [],
    requests = [];
  const returnedModels = new Set();
  const server = createServer(async (req, res) => {
    let body = "";
    for await (const part of req) body += part;
    const value = JSON.parse(body);
    requests.push(value);
    if (requests.length > 4) {
      res.writeHead(429);
      res.end("bounded test request budget exceeded");
      return;
    }
    if (liveEndpoint) {
      try {
        const response = await fetch(`${liveEndpoint}/chat/completions`, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body,
          signal: AbortSignal.timeout(12000),
        });
        res.writeHead(response.status, { "content-type": "text/event-stream" });
        let wire = "";
        for await (const chunk of response.body) {
          wire += new TextDecoder().decode(chunk);
          res.write(chunk);
        }
        for (const line of wire.split("\n")) {
          if (!line.startsWith("data: {")) continue;
          const event = JSON.parse(line.slice(6));
          if (event.model) returnedModels.add(event.model);
        }
        res.end();
      } catch (error) {
        res.destroy(error);
      }
      return;
    }
    res.writeHead(200, { "content-type": "text/event-stream" });
    const toolDone = value.messages.some((m) => m.role === "tool");
    const delta = toolDone
      ? { content: "SCOPED_OK" }
      : {
          tool_calls: [
            {
              index: 0,
              id: "fixture-call",
              type: "function",
              function: {
                name: "mcp__buzz-computer-use__observe",
                arguments: '{"mode":"structured"}',
              },
            },
          ],
        };
    res.end(
      `data: ${JSON.stringify({ id: "test", object: "chat.completion.chunk", choices: [{ index: 0, delta, finish_reason: null }] })}\n\ndata: ${JSON.stringify({ id: "test", object: "chat.completion.chunk", choices: [{ index: 0, delta: {}, finish_reason: toolDone ? "stop" : "tool_calls" }] })}\n\ndata: [DONE]\n\n`,
    );
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  try {
    const home = join(dir, ".dsh-home");
    await mkdir(join(home, "profiles"), { recursive: true });
    await cp(profile, join(home, "profiles/buzz-acp"), {
      recursive: true,
      filter: (p) => !p.includes("node_modules") && !p.includes("/tests"),
    });
    await symlink(nodeModules, join(home, "node_modules"));
    child = spawn(
      process.execPath,
      [resolve(dsh, "../bin.js"), "--profile", "buzz-acp"],
      {
        cwd: dir,
        env: {
          PATH: process.env.PATH,
          HOME: dir,
          DSH_HOME: home,
          DSH_API_KEY: "synthetic-local-test",
          DSH_MODEL: testModel,
          DSH_LLM_BASE_URL: `http://127.0.0.1:${server.address().port}/v1`,
        },
        stdio: ["pipe", "pipe", "pipe"],
      },
    );
    child.stderr.on("data", (b) => {
      stderr += b;
    });
    const lines = createInterface({ input: child.stdout });
    lines.on("line", (line) => {
      const r = JSON.parse(line);
      if (r.id !== undefined) {
        pending.get(r.id)?.(r);
        pending.delete(r.id);
      } else updates.push(r);
    });
    const rpc = (method, params) =>
      new Promise((resolve, reject) => {
        const id = ++sequence;
        const timer = setTimeout(
          () => reject(new Error(`${method} timeout: ${stderr}`)),
          15000,
        );
        pending.set(id, (r) => {
          clearTimeout(timer);
          r.error
            ? reject(new Error(JSON.stringify(r.error)))
            : resolve(r.result);
        });
        child.stdin.write(
          JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n",
        );
      });
    const init = await rpc("initialize", {
      protocolVersion: 1,
      clientCapabilities: {},
    });
    assert.equal(init.agentInfo.name, "buzz-dsh-acp");
    const mcpServers = [
      {
        name: "buzz-computer-use",
        command: process.execPath,
        args: [join(profile, "tests/mcp-fixture.mjs")],
        env: [],
      },
    ];
    await assert.rejects(
      rpc("session/new", {
        cwd: dir,
        mcpServers: [
          {
            ...mcpServers[0],
            env: [{ name: "MCP_FIXTURE_LIST_LOOP", value: "1" }],
          },
        ],
      }),
      /Internal error/,
    );
    await assert.rejects(
      rpc("session/new", {
        cwd: dir,
        mcpServers: [
          mcpServers[0],
          {
            name: "broken",
            command: "/nonexistent/buzz-mcp-test",
            args: [],
            env: [],
          },
        ],
      }),
      /Internal error/,
    );
    const a = await rpc("session/new", { cwd: dir, mcpServers });
    const b = await rpc("session/new", { cwd: dir, mcpServers });
    await assert.rejects(
      rpc("session/set_model", { sessionId: a.sessionId, modelId: "local-v4" }),
      /unknown model/,
    );
    const result = await rpc("session/prompt", {
      sessionId: a.sessionId,
      prompt: [
        {
          type: "text",
          text: "Call mcp__buzz-computer-use__observe exactly once with mode structured. Then reply SCOPED_OK. Do not use other tools.",
        },
      ],
    });
    assert.equal(result.stopReason, "end_turn");
    assert.ok(requests.length >= 2);
    assert.ok(requests.every((r) => r.model === testModel));
    if (liveEndpoint) {
      assert.deepEqual([...returnedModels], [testModel]);
      t.diagnostic(
        `provider-returned model=${[...returnedModels].join(",")}; requests=${requests.length}`,
      );
    }
    assert.ok(
      requests.some((r) =>
        r.messages.some(
          (m) =>
            m.role === "tool" &&
            JSON.stringify(m).includes("STRUCTURED_ONLY_7D"),
        ),
      ),
    );
    assert.ok(
      updates.some((r) => r.params?.update?.sessionUpdate === "tool_call"),
    );
    assert.ok(updates.some((r) => r.params?.update?.status === "completed"));
    assert.ok(
      updates.every(
        (r) => !r.params?.sessionId || r.params.sessionId === a.sessionId,
      ),
    );
    await rpc("session/close", { sessionId: a.sessionId });
    await assert.rejects(
      rpc("session/prompt", {
        sessionId: a.sessionId,
        prompt: [{ type: "text", text: "closed" }],
      }),
      /unknown session/,
    );
    await rpc("session/close", { sessionId: b.sessionId });
  } finally {
    child?.stdin.end();
    child?.kill("SIGTERM");
    if (child && child.exitCode === null)
      await new Promise((resolve) => child.once("exit", resolve));
    await new Promise((resolve) => server.close(resolve));
    await rm(dir, { recursive: true, force: true });
  }
});
