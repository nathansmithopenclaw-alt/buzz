import assert from "node:assert/strict";
import test from "node:test";

import { formatManagedAgentReapNotice } from "./useAgentsDataRefresh.ts";

test("dead-instance reap reports are human-readable", () => {
  assert.deepEqual(
    formatManagedAgentReapNotice({
      instanceId: "xyz.block.buzz.app.dev.proof",
      processCount: 2,
    }),
    {
      title: "Stopped 2 agent processes left by a closed Buzz app",
      description: "Desktop instance: xyz.block.buzz.app.dev.proof",
    },
  );
});

test("dead-instance reap reports use singular process copy", () => {
  assert.equal(
    formatManagedAgentReapNotice({
      instanceId: "xyz.block.buzz.app.dev",
      processCount: 1,
    }).title,
    "Stopped 1 agent process left by a closed Buzz app",
  );
});
