import assert from "node:assert/strict";
import test from "node:test";

import {
  membershipDiff,
  reportFromEvent,
  workflowIdFromEvent,
} from "./summaryProtocol.ts";

test("membershipDiff adds and removes by id", () => {
  assert.deepEqual(membershipDiff(["a", "b"], ["b", "c"]), {
    added: ["c"],
    removed: ["a"],
  });
});

test("reportFromEvent rejects a spoofed reporter pubkey", () => {
  const event = {
    id: "1",
    pubkey: "attacker",
    created_at: 1,
    kind: 40030,
    content: JSON.stringify({
      v: 1,
      run_id: "run",
      title: "phish",
      markdown: "click me",
      status: {},
      period: { since: 1, until: 2 },
      sources: { channels: [], dms: [] },
    }),
    tags: [
      ["t", "buzz-brief"],
      ["p", "owner"],
    ],
  };
  assert.equal(reportFromEvent(event, "reporter"), null);
  assert.equal(reportFromEvent(event, "attacker")?.report.title, "phish");
});

test("workflowIdFromEvent reads the companion workflow tag", () => {
  assert.equal(
    workflowIdFromEvent({
      id: "1",
      pubkey: "aa",
      created_at: 1,
      kind: 30623,
      content: "{}",
      tags: [
        ["d", "summary"],
        ["workflow", "wf-1"],
      ],
    }),
    "wf-1",
  );
});
