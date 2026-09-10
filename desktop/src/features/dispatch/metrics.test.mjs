import test from "node:test";
import assert from "node:assert/strict";
import { dispatchMetrics, medianDuration } from "./metrics.ts";
test("flow metrics exclude unobserved imported intervals and preserve proof stages", () => {
  const issue = (id) => ({
    issueId: id,
    rootCreatedAt: 10,
    status: "in_review",
    enteredStatusAt: 50,
  });
  const context = {
    issues: [issue("native"), issue("imported")],
    activity: [
      { issueId: "imported", import: { system: "linear" }, changes: [] },
    ],
    linkedTasks: [
      { status: "done" },
      { status: "verified" },
      { status: "blocked" },
    ],
  };
  const value = dispatchMetrics(context, 100);
  assert.deepEqual(value.queueAges.in_review, [50]);
  assert.equal(value.importedIssues, 1);
  assert.equal(value.missingStatusHistory, 1);
  assert.equal(value.blockedMissing, 1);
  assert.equal(value.attempts.done, 1);
  assert.equal(value.attempts.verified, 1);
});
test("blocked durations use transitions and do not double count repeated blocked notes", () => {
  const history = [
    { status: "assigned", createdAt: 10 },
    { status: "blocked", createdAt: 20 },
    { status: "blocked", createdAt: 30 },
    { status: "picked_up", createdAt: 50 },
    { status: "blocked", createdAt: 70 },
  ];
  const value = dispatchMetrics(
    { issues: [], activity: [], linkedTasks: [{ status: "blocked", history }] },
    100,
  );
  assert.deepEqual(value.blockedDurations, [30, 30]);
  assert.equal(medianDuration([10, 20, 90, 100]), 55);
  assert.equal(medianDuration([]), null);
});

test("orphan imported history cannot make current native issue counts negative", () => {
  const value = dispatchMetrics({
    issues: [],
    activity: [{ issueId: "missing", import: { system: "linear" } }],
    linkedTasks: [],
  });
  assert.equal(value.nativeIssues, 0);
  assert.equal(value.importedIssues, 0);
});
