import assert from "node:assert/strict";
import test from "node:test";
import { loadLinkedTaskHistory } from "./linkedTaskHistory.ts";

const workspace = crypto.randomUUID();
const channel = crypto.randomUUID();
const viewer = "a".repeat(64);
const task = crypto.randomUUID();
const anchor = {
  id: "1".repeat(64),
  kind: 9,
  created_at: 1,
  tags: [
    ["h", channel],
    ["task", task, "assigned"],
  ],
};
const membership = {
  id: "2".repeat(64),
  kind: 39002,
  created_at: 2,
  tags: [
    ["d", channel],
    ["p", viewer],
  ],
};
const issue = {
  homeChannelId: channel,
  externalRefs: [
    { type: "task", ref: task },
    { type: "task_event", ref: anchor.id },
  ],
};

test("linked tasks fetch their full authorized execution history across channels", async () => {
  const filters = [];
  const history = {
    fetch: async (filter) => {
      filters.push(filter);
      return [anchor];
    },
    fetchExhaustively: async (filter) => {
      filters.push(filter);
      return filter.kinds[0] === 39002
        ? [membership]
        : [anchor, { ...anchor, id: "3".repeat(64) }];
    },
  };
  const result = await loadLinkedTaskHistory(
    history,
    [issue],
    workspace,
    viewer,
  );
  assert.deepEqual(result.channelIds, [channel]);
  assert.equal(result.taskEvents.length, 2);
  const tasks = filters.find((filter) => filter["#task"]);
  assert.deepEqual(tasks["#task"], [task]);
  assert.deepEqual(tasks["#h"], [channel]);
  assert.equal(
    tasks.since,
    undefined,
    "assignment must be included even after a warm cursor",
  );
});

test("latest membership removal prevents execution reads and excludes old cached anchors", async () => {
  const removed = {
    ...membership,
    id: "4".repeat(64),
    created_at: 3,
    tags: [["d", channel]],
  };
  const result = await loadLinkedTaskHistory(
    {
      fetch: async () => [anchor],
      fetchExhaustively: async (filter) => {
        assert.equal(filter.kinds[0], 39002);
        return [membership, removed];
      },
    },
    [issue],
    workspace,
    viewer,
  );
  assert.deepEqual(result.channelIds, []);
  assert.deepEqual(result.taskEvents, []);
  assert.equal(
    result.events.length,
    2,
    "native fold must observe the revocation",
  );
});

test("issues without explicit task references do not scan their conversations", async () => {
  const forbidden = async () => {
    throw new Error("unrelated read");
  };
  const result = await loadLinkedTaskHistory(
    { fetch: forbidden, fetchExhaustively: forbidden },
    [{ ...issue, externalRefs: [] }],
    workspace,
    viewer,
  );
  assert.deepEqual(result, {
    events: [],
    taskEvents: [],
    channelIds: [],
    taskIds: [],
  });
});

test("execution read failure remains an error rather than a false empty history", async () => {
  await assert.rejects(
    loadLinkedTaskHistory(
      {
        fetch: async () => [anchor],
        fetchExhaustively: async (filter) => {
          if (filter.kinds[0] === 39002) return [membership];
          throw new Error("relay unavailable");
        },
      },
      [issue],
      workspace,
      viewer,
    ),
    /relay unavailable/,
  );
});
