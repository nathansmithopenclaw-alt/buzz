import test from "node:test";
import assert from "node:assert/strict";
import { buildSummaryLink, parseSummaryLink } from "./summaryLink.ts";
const id = "ab".repeat(32),
  relay = "wss://relay.example/community";
test("report links bind an exact event and relay", () =>
  assert.deepEqual(parseSummaryLink(buildSummaryLink(id, relay)), {
    id,
    relay,
  }));
test("report links reject ambiguous and unsafe destinations", () => {
  for (const href of [
    "buzz://summary?id=bad&relay=wss://relay.example",
    `buzz://summary?id=${id}&relay=https://relay.example`,
    `buzz://summary?id=${id}&relay=wss://user:password@relay.example`,
    `${buildSummaryLink(id, relay)}&id=${id}`,
    `${buildSummaryLink(id, relay)}#extra`,
    `${buildSummaryLink(id, relay)}&unknown=yes`,
  ])
    assert.equal(parseSummaryLink(href), null, href);
});
