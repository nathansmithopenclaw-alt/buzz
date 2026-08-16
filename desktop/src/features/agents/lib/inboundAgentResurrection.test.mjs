import assert from "node:assert/strict";
import test from "node:test";

import {
  inboundResurrectionTargets,
  inboundWakeRefusalMessage,
  shouldShowInboundWakeRefusal,
} from "./inboundAgentResurrection.ts";

const AGENT = "aa".repeat(32);
const OWNER = "bb".repeat(32);
const OTHER = "cc".repeat(32);

test("surfaces actionable wake refusals without noisy policy skips", () => {
  assert.equal(inboundWakeRefusalMessage("SkipAlreadyRunning"), null);
  assert.equal(inboundWakeRefusalMessage("SkipAuthorGated"), null);
  assert.match(
    inboundWakeRefusalMessage("SkipCrashBackoff"),
    /repeated startup failures/,
  );
  assert.equal(
    inboundWakeRefusalMessage(
      "Observer device cannot control agents; switch this device to Host in Settings",
    ),
    "This device is an Observer, so it cannot wake agents. Switch to Host in Settings to run agents here.",
  );
  assert.equal(
    inboundWakeRefusalMessage("FutureDebugVariant"),
    "This agent did not wake automatically. Start it manually from Agents.",
  );
});

test("rate limits the same wake refusal per agent and reason for one hour", () => {
  assert.equal(shouldShowInboundWakeRefusal(AGENT, "observer", 1_000), true);
  assert.equal(shouldShowInboundWakeRefusal(AGENT, "observer", 2_000), false);
  assert.equal(
    shouldShowInboundWakeRefusal(AGENT, "observer", 3_601_000),
    true,
  );
  assert.equal(shouldShowInboundWakeRefusal(AGENT, "different", 2_000), true);
});

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
