import test from "node:test";
import assert from "node:assert/strict";
import {
  equivalentPendingIssueWrite,
  issueWriteAccepted,
} from "./issueWriteRecovery.ts";
const event = (input = {}) => ({
  kind: 40021,
  pubkey: "a",
  id: "original",
  tags: [
    ["h", "workspace"],
    ["request", "request"],
  ],
  content: JSON.stringify({
    issue_id: "issue",
    parent_event: "tip",
    request_id: "request",
    set: { title: "Fixed" },
  }),
  ...input,
});
test("uncertain issue edits reuse only matching content, author, workspace and predecessor", () => {
  const a = event(),
    b = event({
      id: "new",
      created_at: 99,
      content: JSON.stringify({
        issue_id: "issue",
        parent_event: "tip",
        request_id: "new-request",
        set: { title: "Fixed" },
      }),
      tags: [
        ["request", "new-request"],
        ["h", "workspace"],
      ],
    });
  assert.equal(equivalentPendingIssueWrite(a, b), true);
  assert.equal(
    equivalentPendingIssueWrite(a, { ...b, pubkey: "other" }),
    false,
  );
  assert.equal(
    equivalentPendingIssueWrite(a, { ...b, tags: [["h", "other"]] }),
    false,
  );
  assert.equal(
    equivalentPendingIssueWrite(a, {
      ...b,
      content: b.content.replace('"tip"', '"new-tip"'),
    }),
    false,
  );
  assert.equal(
    equivalentPendingIssueWrite({ ...a, kind: 40020 }, { ...b, kind: 40020 }),
    false,
  );
});
test("relay publication alone never confirms an issue or comment write", () => {
  const context = { issues: [], activity: [], comments: [] };
  assert.equal(issueWriteAccepted(context, event()), false);
  context.activity.push({ eventId: "original" });
  assert.equal(issueWriteAccepted(context, event()), true);
});
