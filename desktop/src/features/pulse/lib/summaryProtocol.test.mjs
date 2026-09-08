import assert from "node:assert/strict";
import test from "node:test";

import {
  briefConfigFilter,
  conversationsFromChannels,
  briefReportsFilter,
  membershipDiff,
  reportFromEvent,
  workflowIdFromEvent,
} from "./summaryProtocol.ts";

test("briefConfigFilter pins #p so the p-gate does not close the owner's read", () => {
  const filter = briefConfigFilter("owner");
  assert.deepEqual(filter["#p"], ["owner"]);
  assert.deepEqual(filter.authors, ["owner"]);
  assert.deepEqual(filter["#d"], ["summary"]);
  assert.deepEqual(filter.kinds, [30623]);
});

test("briefReportsFilter pins #p to the owner and authors to the reporter", () => {
  const filter = briefReportsFilter("owner", "reporter");
  assert.deepEqual(filter["#p"], ["owner"]);
  assert.deepEqual(filter.authors, ["reporter"]);
  assert.deepEqual(filter.kinds, [40030]);
});

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

test("Summary DM labels resolve participants and omit the owner", () => {
  const owner = "11".repeat(32),
    first = "22".repeat(32),
    second = "33".repeat(32);
  const room = {
    id: "dm",
    name: "DM",
    channelType: "dm",
    isMember: true,
    archivedAt: null,
    participants: ["Nathan", "Old first", "Old second"],
    participantPubkeys: [owner, first, second],
  };
  const profiles = {
    [first]: { pubkey: first, displayName: "Grok" },
    [second]: { pubkey: second, displayName: "Nectar" },
  };
  assert.equal(
    conversationsFromChannels([room], owner, profiles)[0].name,
    "Grok, Nectar",
  );
  assert.equal(
    conversationsFromChannels([room], owner)[0].name,
    "Old first, Old second",
  );
});
