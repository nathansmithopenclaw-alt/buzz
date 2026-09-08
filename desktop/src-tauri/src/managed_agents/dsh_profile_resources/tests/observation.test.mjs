import test from "node:test";
import assert from "node:assert/strict";
import { guardObservations } from "../packages/dsh-acp-buzz/observation.js";
test("compaction invalidates each browser independently and ignores sibling observations", () => {
  const events = new Map(),
    guards = [];
  const owner = { session: {} },
    sibling = { session: {} };
  guardObservations(
    {
      on: (name, handler) => events.set(name, handler),
      tools: { guard: (handler) => guards.push(handler) },
    },
    owner,
  );
  const exec = (agent, name, session) => ({
    agent,
    name: `mcp__buzz-computer-use__${name}`,
    arguments: { session },
  });
  const denied = (session) => guards[0](exec(owner, "browser_action", session));
  events.get("session/event")(owner.session, { type: "compaction/summary" });
  assert.match(denied("a"), /fresh observation/);
  events.get("tools/result")(exec(sibling, "browser_observe", "a"), {
    isError: false,
  });
  assert.match(denied("a"), /fresh observation/);
  events.get("tools/result")(exec(owner, "browser_observe", "a"), {
    isError: true,
  });
  assert.match(denied("a"), /fresh observation/);
  events.get("tools/result")(exec(owner, "browser_observe", "a"), {
    isError: false,
  });
  assert.equal(denied("a"), undefined);
  assert.match(denied("b"), /fresh observation/);
  events.get("session/event")(owner.session, { type: "compaction/prune" });
  assert.match(denied("a"), /fresh observation/);
});
