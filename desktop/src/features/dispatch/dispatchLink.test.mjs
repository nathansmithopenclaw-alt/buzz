import test from "node:test";
import assert from "node:assert/strict";
import { buildDispatchLink, parseDispatchLink } from "./dispatchLink.ts";
test("Dispatch links preserve exact community and issue scope", () => {
  const input = {
    workspace: crypto.randomUUID(),
    issue: crypto.randomUUID(),
    relay: "wss://relay.example/",
  };
  const link = buildDispatchLink(input);
  assert.deepEqual(parseDispatchLink(link), input);
  for (const href of [
    `${link}&issue=${crypto.randomUUID()}`,
    `${link}#fragment`,
    link.replace("dispatch?", "dispatch/extra?"),
    link.replace("wss%3A%2F%2F", "https%3A%2F%2F"),
    link.replace("relay.example", "u%3Ap%40relay.example"),
  ])
    assert.equal(parseDispatchLink(href), null);
});
