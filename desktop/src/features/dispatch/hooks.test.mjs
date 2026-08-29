import assert from "node:assert/strict";
import test from "node:test";

import { MAX_FILTERS_PER_REQ } from "../../shared/api/relayClientShared.ts";
import {
  probePopulatedWorkspaces,
  TRACKER_HISTORY_KINDS,
  TRACKER_KINDS,
} from "./hooks.ts";

function workspaceIds(count) {
  return Array.from({ length: count }, (_, index) => `ws-${index}`);
}

function trackerEvent(...channels) {
  return {
    id: "a".repeat(64),
    pubkey: "b".repeat(64),
    created_at: 1,
    kind: 40020,
    tags: channels.map((channel) => ["h", channel]),
    content: "",
    sig: "c".repeat(128),
  };
}

test("Dispatch live and populated-probe kinds exclude unused project activity", () => {
  assert.deepEqual(TRACKER_KINDS, [40020, 40021, 40022]);
  assert.equal(TRACKER_KINDS.includes(40025), false);
});

test("Dispatch cold-load history kinds already skip project activity", () => {
  assert.deepEqual(TRACKER_HISTORY_KINDS, [40020, 40021]);
  assert.equal(TRACKER_HISTORY_KINDS.includes(40025), false);
});

test("populated probe costs one request per filter batch, not one per workspace", async () => {
  const ids = workspaceIds(31);
  const batches = [];
  const populated = await probePopulatedWorkspaces(
    {
      fetchMulti: async (filters) => {
        batches.push(filters);
        return [];
      },
    },
    ids,
  );

  assert.equal(batches.length, Math.ceil(31 / MAX_FILTERS_PER_REQ));
  assert.equal(
    batches.every((filters) => filters.length <= MAX_FILTERS_PER_REQ),
    true,
    "a batch over the relay cap is rejected as a protocol error",
  );
  assert.deepEqual(
    batches.flat().map((filter) => filter["#h"][0]),
    ids,
    "every workspace is probed exactly once, in order",
  );
  assert.equal(populated.size, 0);
});

test("each workspace keeps its own limit so a busy one cannot starve the rest", async () => {
  const ids = workspaceIds(3);
  const seen = [];
  await probePopulatedWorkspaces(
    {
      fetchMulti: async (filters) => {
        seen.push(...filters);
        return [];
      },
    },
    ids,
  );

  assert.deepEqual(
    seen.map((filter) => ({ h: filter["#h"], limit: filter.limit })),
    ids.map((id) => ({ h: [id], limit: 1 })),
    "one narrow filter per workspace — never a single combined #h filter",
  );
  assert.deepEqual(seen[0].kinds, TRACKER_KINDS);
});

test("populated workspaces are attributed from the event's own h tag", async () => {
  // The relay does not report which filter matched, so a returned event is
  // credited to every probed workspace it is tagged with.
  const populated = await probePopulatedWorkspaces(
    {
      fetchMulti: async () => [
        trackerEvent("ws-1"),
        trackerEvent("ws-2", "ws-3"),
        trackerEvent("not-probed"),
      ],
    },
    ["ws-0", "ws-1", "ws-2", "ws-3"],
  );

  assert.deepEqual([...populated].sort(), ["ws-1", "ws-2", "ws-3"]);
  assert.equal(populated.has("ws-0"), false, "a silent empty stays empty");
  assert.equal(populated.has("not-probed"), false);
});

test("no workspaces means no probe request at all", async () => {
  let calls = 0;
  const populated = await probePopulatedWorkspaces(
    {
      fetchMulti: async () => {
        calls += 1;
        return [];
      },
    },
    [],
  );

  assert.equal(calls, 0);
  assert.equal(populated.size, 0);
});
