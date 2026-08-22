import assert from "node:assert/strict";
import test from "node:test";

import { TRACKER_HISTORY_KINDS, TRACKER_KINDS } from "./hooks.ts";

test("Dispatch live and populated-probe kinds exclude unused project activity", () => {
  assert.deepEqual(TRACKER_KINDS, [40020, 40021, 40022]);
  assert.equal(TRACKER_KINDS.includes(40025), false);
});

test("Dispatch cold-load history kinds already skip project activity", () => {
  assert.deepEqual(TRACKER_HISTORY_KINDS, [40020, 40021]);
  assert.equal(TRACKER_HISTORY_KINDS.includes(40025), false);
});
