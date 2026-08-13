import assert from "node:assert/strict";
import test from "node:test";

import {
  blockedIssueIds,
  EMPTY_DISPATCH_FILTERS,
  filterDispatchIssues,
  groupDispatchIssues,
  shortPubkey,
  sortDispatchBoardIssues,
  sortDispatchIssues,
} from "./viewModel.ts";

function issue(overrides) {
  return {
    streamId: "00000000-0000-4000-8000-000000000001",
    issueId: crypto.randomUUID(),
    rootEvent: "1".repeat(64),
    rootCreatedAt: 1,
    tipEvent: "2".repeat(64),
    tipCreatedAt: 1,
    revision: 0,
    enteredStatusAt: 1,
    identifier: "SMI-1",
    title: "Dispatch fixture",
    body: "fixture body",
    status: "backlog",
    priority: null,
    assignee: null,
    projectId: null,
    due: null,
    estimateMinutes: null,
    homeChannelId: null,
    workflow: "default",
    labels: [],
    externalRefs: [],
    relations: [],
    ...overrides,
  };
}

test("filters mirror Tracker query fields without consulting event history", () => {
  const issues = [
    issue({
      identifier: "SMI-1",
      status: "in_progress",
      priority: "P1",
      labels: ["desktop"],
    }),
    issue({
      identifier: "SMI-2",
      title: "Relay repair",
      status: "triage",
      priority: null,
    }),
  ];
  assert.deepEqual(
    filterDispatchIssues(issues, {
      ...EMPTY_DISPATCH_FILTERS,
      status: "in_progress",
      priority: "P1",
      label: "desktop",
      query: "fixture",
    }).map((value) => value.identifier),
    ["SMI-1"],
  );
  assert.deepEqual(
    filterDispatchIssues(issues, {
      ...EMPTY_DISPATCH_FILTERS,
      priority: "none",
    }).map((value) => value.identifier),
    ["SMI-2"],
  );
});

test("grouping uses explicit none buckets for missing assignee and project", () => {
  const issues = [
    issue({ identifier: "SMI-1" }),
    issue({ identifier: "SMI-2", assignee: "a".repeat(64) }),
  ];
  assert.deepEqual(
    groupDispatchIssues(issues, "assignee", [], {}).map((group) => [
      group.id,
      group.issues.length,
    ]),
    [
      ["a".repeat(64), 1],
      ["none", 1],
    ],
  );
});

test("list sorting is stable and long identifiers have a bounded fallback", () => {
  const long = `D-${"a".repeat(32)}`;
  const issues = [
    issue({ identifier: "SMI-20", title: "Zulu" }),
    issue({ identifier: long, title: "Alpha" }),
  ];
  assert.deepEqual(
    sortDispatchIssues(issues, { key: "title", direction: "asc" }, [], {}).map(
      (value) => value.title,
    ),
    ["Alpha", "Zulu"],
  );
  assert.equal(shortPubkey("f".repeat(64)), "ffffffff…ffff");
});

test("board order is P0 first, then most recently updated within priority", () => {
  const issues = [
    issue({ identifier: "SMI-3", priority: "P1", tipCreatedAt: 300 }),
    issue({ identifier: "SMI-2", priority: "P0", tipCreatedAt: 100 }),
    issue({ identifier: "SMI-1", priority: "P0", tipCreatedAt: 200 }),
    issue({ identifier: "SMI-4", priority: null, tipCreatedAt: 400 }),
  ];
  assert.deepEqual(
    sortDispatchBoardIssues(issues, [], {}).map((value) => value.identifier),
    ["SMI-1", "SMI-2", "SMI-3", "SMI-4"],
  );
  assert.deepEqual(
    sortDispatchIssues(
      issues,
      { key: "priority", direction: "asc" },
      [],
      {},
    ).map((value) => value.identifier),
    ["SMI-1", "SMI-2", "SMI-3", "SMI-4"],
  );
});

test("blocked indicator is derived from folded relation projections", () => {
  const target = crypto.randomUUID();
  const source = issue({
    relations: [
      {
        kind: "blocks",
        targetIssueId: target,
        targetRootEvent: "3".repeat(64),
      },
    ],
  });
  assert.deepEqual([...blockedIssueIds([source])], [target]);
});
