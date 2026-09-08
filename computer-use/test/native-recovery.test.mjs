import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import { NativeService } from "../src/native.mjs";
import { computerServer } from "../src/mcp.mjs";

async function fixture(t, callTool) {
  const registry = await mkdtemp("/tmp/buzz-native-recovery-");
  const native = new NativeService({
    driver: "fixture",
    socket: "fixture",
    registry,
  });
  native.connect = async () => ({ callTool });
  const server = computerServer({ available: () => true }, native);
  const client = new Client({ name: "content-only-acp", version: "1" });
  const [a, b] = InMemoryTransport.createLinkedPair();
  await server.connect(a);
  await client.connect(b);
  t.after(async () => {
    await client.close();
    await server.close();
    await native.shutdown();
    await rm(registry, { recursive: true, force: true });
  });
  const call = (name, args = {}) => client.callTool({ name, arguments: args });
  const text = (result) =>
    result.content
      .filter((b) => b.type === "text")
      .map((b) => b.text)
      .join("\n");
  return { native, call, text };
}

test("MCP errors retain driver evidence and give content-only adapters truthful lease recovery", async (t) => {
  const image = { type: "image", mimeType: "image/png", data: "aW1hZ2U=" };
  let calls = 0;
  const { call, text } = await fixture(t, async ({ name }) => {
    calls++;
    if (name === "click")
      return {
        isError: true,
        structuredContent: {
          status: "refused",
          refusal: { code: "invalid_snapshot_id" },
        },
        content: [
          { type: "text", text: "click: snapshot_id has invalid format" },
          image,
        ],
      };
    return {
      structuredContent: {
        snapshot_id: `s0000000${calls}`,
        elements: [{ element_token: `s0000000${calls}:1` }],
      },
      content: [image],
    };
  });
  const unowned = await call("native_click");
  assert.equal(unowned.isError, true);
  assert.match(text(unowned), /"acquired":false/);
  await call("native_acquire");
  const first = await call("native_get_window_state");
  assert.match(text(first), /s00000001:1/);
  assert.deepEqual(
    first.content.find((b) => b.type === "image"),
    image,
  );
  const refused = await call("native_click", { snapshot_id: "s0" });
  assert.equal(refused.isError, true);
  assert.match(text(refused), /invalid_snapshot_id/);
  assert.match(text(refused), /snapshot_id has invalid format/);
  assert.match(text(refused), /"acquired":true/);
  assert.match(text(refused), /"observation_required":true/);
  assert.deepEqual(
    refused.content.find((b) => b.type === "image"),
    image,
  );
  const stale = await call("native_type_text", {
    text: "must not reach driver",
  });
  assert.equal(stale.isError, true);
  assert.match(text(stale), /native_get_window_state/);
  assert.equal(calls, 2);
  const fresh = await call("native_get_window_state");
  assert.match(text(fresh), /s00000003:1/);
  await call("native_release");
  const status = JSON.parse(text(await call("computer_status")));
  assert.equal(status.nativeState.acquired, false);
});

test("unknown native outcome fences retries, reacquisition, observations and lease release through MCP", async (t) => {
  let calls = 0;
  const { native, call, text } = await fixture(t, async ({ name }) => {
    calls++;
    if (name === "set_value")
      throw new Error("transport disconnected after dispatch");
    return { content: [] };
  });
  await call("native_acquire");
  await call("native_get_window_state");
  const failure = await call("native_set_value", {
    value: "possible side effect",
  });
  assert.equal(failure.isError, true);
  assert.match(text(failure), /transport disconnected after dispatch/);
  assert.match(text(failure), /"acquired":true/);
  assert.match(text(failure), /"outcome_uncertain":true/);
  assert.equal(native.observed, false);
  for (const name of [
    "native_set_value",
    "native_get_window_state",
    "native_acquire",
    "native_release",
  ]) {
    assert.equal((await call(name)).isError, true, name);
  }
  assert.equal(calls, 2, "no driver call may follow an unknown outcome");
  native.lastUsed = Date.now() - 61_000;
  await native.reap();
  const status = JSON.parse(text(await call("computer_status")));
  assert.equal(status.nativeState.acquired, true);
  assert.equal(status.nativeState.outcome_uncertain, true);
});

test("failed observations cannot leave a previous snapshot eligible for input", async (t) => {
  let calls = 0;
  const { call } = await fixture(t, async () => {
    calls++;
    return { isError: calls === 2, content: [] };
  });
  await call("native_acquire");
  await call("native_get_window_state");
  assert.equal((await call("native_get_window_state")).isError, true);
  assert.equal((await call("native_click")).isError, true);
  assert.equal(calls, 2);
});
