import assert from "node:assert/strict";
import { test } from "node:test";
import { loadAdmissionSources } from "./admissionHistory.ts";
const workspace = "workspace",
  sourceId = "a".repeat(64);
const receipt = {
  id: "b".repeat(64),
  kind: 40027,
  created_at: 100,
  tags: [
    ["h", workspace],
    ["e", sourceId],
  ],
};
test("a new receipt loads the exact older source beyond a warm timestamp cursor", async () => {
  const source = {
    id: sourceId,
    kind: 40021,
    created_at: 1,
    tags: [["h", workspace]],
  };
  let request;
  const values = await loadAdmissionSources(
    {
      fetch: async (filter) => {
        request = filter;
        return [source];
      },
    },
    [receipt],
    workspace,
    new Set(),
  );
  assert.deepEqual(request.ids, [sourceId]);
  assert.deepEqual(request["#h"], [workspace]);
  assert.equal(request.since, undefined);
  assert.deepEqual(values, [source]);
});
test("known sources and foreign-workspace receipts do not cause reads", async () => {
  await loadAdmissionSources(
    {
      fetch: async () => {
        throw Error("unexpected read");
      },
    },
    [receipt],
    workspace,
    new Set([sourceId]),
  );
  await loadAdmissionSources(
    {
      fetch: async () => {
        throw Error("unexpected read");
      },
    },
    [receipt],
    "elsewhere",
    new Set(),
  );
});
test("missing admitted source is a visible history error", async () => {
  await assert.rejects(
    loadAdmissionSources(
      { fetch: async () => [] },
      [receipt],
      workspace,
      new Set(),
    ),
    /unavailable/,
  );
});
