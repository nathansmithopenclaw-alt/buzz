import assert from "node:assert/strict";
import test from "node:test";

import {
  chronologicalActivity,
  selectMyIssues,
  splitTriageIssues,
  taskMessageLink,
} from "./selectors.mjs";

function issue(overrides = {}) {
  return {
    issueId: crypto.randomUUID(),
    identifier: "SC-1",
    status: "triage",
    priority: null,
    assignee: null,
    enteredStatusAt: 10,
    due: null,
    ...overrides,
  };
}

test("triage splits assignment and orders P0 before oldest status epoch", () => {
  const p0New = issue({
    identifier: "P0-new",
    priority: "P0",
    enteredStatusAt: 30,
  });
  const p1Old = issue({
    identifier: "P1-old",
    priority: "P1",
    enteredStatusAt: 1,
  });
  const noneOld = issue({ identifier: "none-old", enteredStatusAt: 0 });
  const assigned = issue({ identifier: "assigned", assignee: "a".repeat(64) });
  const result = splitTriageIssues([p1Old, assigned, noneOld, p0New]);
  assert.deepEqual(
    result.unassigned.map((item) => item.identifier),
    ["P0-new", "P1-old", "none-old"],
  );
  assert.deepEqual(
    result.assigned.map((item) => item.identifier),
    ["assigned"],
  );
});

test("my issues selects self, excludes terminal, then orders priority and due", () => {
  const self = "A".repeat(64);
  const result = selectMyIssues(
    [
      issue({
        identifier: "terminal",
        assignee: self,
        status: "done",
        priority: "P0",
      }),
      issue({ identifier: "other", assignee: "b".repeat(64), priority: "P0" }),
      issue({
        identifier: "p1-late",
        assignee: self,
        priority: "P1",
        due: { value: 20 },
      }),
      issue({
        identifier: "p1-early",
        assignee: self.toLowerCase(),
        priority: "P1",
        due: { value: 10 },
      }),
      issue({ identifier: "p0", assignee: self, priority: "P0" }),
    ],
    self,
  );
  assert.deepEqual(
    result.map((item) => item.identifier),
    ["p0", "p1-early", "p1-late"],
  );
});

test("activity and comments are chronologically interleaved", () => {
  const timeline = chronologicalActivity(
    [{ eventId: "b", createdAt: 2 }],
    [
      { eventId: "a", createdAt: 1 },
      { eventId: "c", createdAt: 2 },
    ],
  );
  assert.deepEqual(
    timeline.map((item) => item.eventId),
    ["a", "b", "c"],
  );
});

test("linked tasks use the canonical message deep-link shape", () => {
  assert.equal(
    taskMessageLink({ channelId: "workspace", eventId: "event" }),
    "buzz://message?channel=workspace&id=event",
  );
});
