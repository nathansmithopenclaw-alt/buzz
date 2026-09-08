import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { Context } from "@deepseek-ai/cordis";
import SystemPrompt from "@deepseek-ai/dsh-system-prompt";
import Tools from "@deepseek-ai/dsh-tools";
import { createScope } from "@deepseek-ai/dsh-scope";
import {
  setupSession,
  validateServers,
} from "../packages/dsh-acp-buzz/session.js";

const fixture = fileURLToPath(new URL("./mcp-fixture.mjs", import.meta.url));
const name = "mcp__buzz-computer-use__observe";
const server = {
  name: "buzz-computer-use",
  command: process.execPath,
  args: [fixture],
  env: [{ name: "MCP_FIXTURE_GRANDCHILD", value: "1" }],
};
test("real Cordis scopes isolate same-name MCP servers and dispose registrations/processes", {
  timeout: 15000,
}, async () => {
  const root = new Context();
  let ctx;
  const promptPlugin = root.plugin(SystemPrompt);
  await promptPlugin;
  const toolsPlugin = root.plugin(Tools);
  await toolsPlugin;
  const access = root.inject(["tools"], (c) => {
    ctx = c;
  });
  await access;
  const a = { id: "a" },
    b = { id: "b" };
  const sa = createScope(ctx, a),
    sb = createScope(ctx, b);
  // AgentLoop normally installs this scope-local agent service.

  let pids = [];
  try {
    await setupSession(sa.ctx, validateServers([server]), process.cwd(), {
      model: "test",
      provider: "test",
    });
    await setupSession(sb.ctx, validateServers([server]), process.cwd(), {
      model: "test",
      provider: "test",
    });
    assert.equal(ctx.tools.get(name), undefined);
    assert.ok(ctx.tools.get(name, a));
    assert.ok(ctx.tools.get(name, b));
    const call = (agent) =>
      ctx.tools.execute({
        callId: agent.id,
        name,
        agent,
        arguments: { mode: "structured" },
        signal: new AbortController().signal,
      });
    const [ra, rb] = await Promise.all([call(a), call(b)]);
    assert.equal(ra.isError, false, JSON.stringify(ra));
    assert.equal(rb.isError, false, JSON.stringify(rb));
    pids = [ra.value.structuredContent.pid, rb.value.structuredContent.pid];
    assert.notEqual(pids[0], pids[1]);
    const grandchildPids = [
      ra.value.structuredContent.grandchildPid,
      rb.value.structuredContent.grandchildPid,
    ];
    assert.match(JSON.stringify(ra.content), /STRUCTURED_ONLY_7D/);
    const child = { id: "child" };
    const inherited = createScope(ctx, child, { parent: a });
    try {
      const denied = await call(child);
      assert.equal(denied.isError, true);
      assert.match(JSON.stringify(denied), /exact ACP agent/);
    } finally {
      await inherited.dispose();
    }
    const rich = await ctx.tools.execute({
      callId: "image",
      name,
      agent: b,
      arguments: { mode: "image" },
      signal: new AbortController().signal,
    });
    assert.equal(rich.isError, false);
    assert.match(JSON.stringify(rich.content), /image unavailable/);
    assert.match(JSON.stringify(rich.content), /STRUCTURED_ONLY_7D/);
    assert.ok(!JSON.stringify(rich.content).includes("AA=="));
    const controller = new AbortController();
    const hanging = ctx.tools.execute({
      callId: "hang",
      name,
      agent: a,
      arguments: { mode: "hang" },
      signal: controller.signal,
    });
    setTimeout(() => controller.abort(new Error("test cancellation")), 50);
    assert.equal((await hanging).isError, true);
    await sa.dispose();
    assert.equal(ctx.tools.get(name, a), undefined);
    assert.throws(() => process.kill(pids[0], 0), { code: "ESRCH" });
    assert.equal((await call(b)).isError, false);
    await sb.dispose();
    assert.throws(() => process.kill(pids[1], 0), { code: "ESRCH" });
    for (const pid of grandchildPids)
      assert.throws(() => process.kill(pid, 0), { code: "ESRCH" });
  } finally {
    await sa.dispose();
    await sb.dispose();
    await access.dispose();
    await toolsPlugin.dispose();
    await promptPlugin.dispose();
  }
});
test("whole batch rejects invalid/duplicate transports before setup", () => {
  assert.throws(() => validateServers([server, server]), /duplicate/);
  assert.throws(
    () => validateServers([{ ...server, command: "node" }]),
    /absolute/,
  );
  assert.throws(() => validateServers([{ ...server, type: "http" }]), /stdio/);
});
