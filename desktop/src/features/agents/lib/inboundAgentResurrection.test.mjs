import assert from "node:assert/strict";
import test from "node:test";

import { inboundResurrectionTargets } from "./inboundAgentResurrection.ts";

const AGENT = "aa".repeat(32);
const OWNER = "bb".repeat(32);
const OTHER = "cc".repeat(32);

function event(overrides = {}) {
  return {
    id: "evt1",
    pubkey: OWNER,
    kind: 9,
    created_at: 1,
    content: "hello",
    tags: [
      ["p", AGENT],
      ["h", "channel"],
    ],
    ...overrides,
  };
}

function agent(overrides = {}) {
  return {
    pubkey: AGENT,
    backend: { type: "local" },
    status: "stopped",
    ...overrides,
  };
}

test("targets a stopped local agent mentioned by someone else", () => {
  assert.deepEqual(inboundResurrectionTargets(event(), [agent()]), [AGENT]);
});

test("ignores running agents", () => {
  assert.deepEqual(
    inboundResurrectionTargets(event(), [agent({ status: "running" })]),
    [],
  );
});

test("ignores provider backends", () => {
  assert.deepEqual(
    inboundResurrectionTargets(event(), [
      agent({ backend: { type: "provider", id: "x", config: {} } }),
    ]),
    [],
  );
});

test("ignores events that do not mention the agent", () => {
  assert.deepEqual(
    inboundResurrectionTargets(event({ tags: [["p", OTHER]] }), [agent()]),
    [],
  );
});

test("ignores the agent mentioning itself", () => {
  assert.deepEqual(
    inboundResurrectionTargets(event({ pubkey: AGENT }), [agent()]),
    [],
  );
});

test("ignores non-message kinds", () => {
  assert.deepEqual(
    inboundResurrectionTargets(event({ kind: 7 }), [agent()]),
    [],
  );
});
