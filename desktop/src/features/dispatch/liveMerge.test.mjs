import assert from "node:assert/strict";
import test from "node:test";

import { createDispatchLiveMerge } from "./liveMerge.ts";

function event(id, kind = 40021) {
  return {
    id: id.repeat(64),
    pubkey: "a".repeat(64),
    created_at: 100,
    kind,
    tags: [],
    content: "",
    sig: "b".repeat(128),
  };
}

function timerHost() {
  let nextId = 0;
  const timers = new Map();
  return {
    setTimeout(handler, milliseconds) {
      const id = ++nextId;
      timers.set(
        id,
        globalThis.setTimeout(() => {
          timers.delete(id);
          handler();
        }, milliseconds),
      );
      return id;
    },
    clearTimeout(id) {
      const timer = timers.get(id);
      if (timer) globalThis.clearTimeout(timer);
      timers.delete(id);
    },
  };
}

test("live events wait for the cold fold and merge in one deduplicated batch", async () => {
  const batches = [];
  const folded = [];
  const merger = createDispatchLiveMerge({
    delayMs: 0,
    timerHost: timerHost(),
    now: () => 123,
    fold: async (batch) => {
      batches.push(batch);
      return { revision: batches.length };
    },
    onFolded: (value) => folded.push(value),
    onError: assert.fail,
  });

  merger.push(event("1"));
  merger.push(event("1"));
  merger.push(event("2", 9));
  await new Promise((resolve) => setTimeout(resolve, 5));
  assert.equal(batches.length, 0);

  merger.setColdLoadReady(true);
  await new Promise((resolve) => setTimeout(resolve, 5));

  assert.equal(batches.length, 1);
  assert.deepEqual(
    batches[0].events.map(({ id }) => id),
    ["1".repeat(64)],
  );
  assert.deepEqual(
    batches[0].taskEvents.map(({ id }) => id),
    ["2".repeat(64)],
  );
  assert.equal(batches[0].snapshotAt, 123);
  assert.deepEqual(folded, [{ revision: 1 }]);
  merger.cancel();
});

test("incremental folds are serialized in arrival order", async () => {
  const releases = [];
  const started = [];
  const folded = [];
  const merger = createDispatchLiveMerge({
    delayMs: 0,
    timerHost: timerHost(),
    fold: (batch) => {
      started.push(batch.events[0].id);
      return new Promise((resolve) => releases.push(resolve));
    },
    onFolded: (value) => folded.push(value),
    onError: assert.fail,
  });
  merger.setColdLoadReady(true);
  merger.push(event("3"));
  await new Promise((resolve) => setTimeout(resolve, 5));
  merger.push(event("4"));
  await new Promise((resolve) => setTimeout(resolve, 5));

  assert.deepEqual(started, ["3".repeat(64)]);
  releases.shift()("first");
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(started, ["3".repeat(64), "4".repeat(64)]);
  releases.shift()("second");
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(folded, ["first", "second"]);
  merger.cancel();
});
