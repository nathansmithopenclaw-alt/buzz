import test from "node:test";
import assert from "node:assert/strict";
import { authorizationIntent } from "./relayAuthorizationIntent.ts";

test("native key ordering does not turn an accepted authorization into a new edit", () => {
  const original = {
    type: "autopilot",
    enabled: false,
    agents: ["a", "b"],
    activation_id: "activation",
    confirmation_event: null,
  };
  const native = Object.fromEntries(Object.entries(original).sort());
  assert.equal(authorizationIntent(original), authorizationIntent(native));
  for (const change of [
    { enabled: true },
    { agents: ["b", "a"] },
    { activation_id: "new" },
    { confirmation_event: "third" },
  ])
    assert.notEqual(
      authorizationIntent(original),
      authorizationIntent({ ...native, ...change }),
    );
});
