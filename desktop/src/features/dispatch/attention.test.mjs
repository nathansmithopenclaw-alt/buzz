import test from "node:test";
import assert from "node:assert/strict";
import { selectAttention, routingSuggestions } from "./attention.ts";
import { activeRecords } from "./recordTypes.ts";
import {
  dispatchContextExport,
  relatedIssueSuggestions,
} from "./contextExport.ts";
import { selectMyIssues } from "./selectors.mjs";
const issue = (id, status = "todo") => ({
  issueId: id,
  streamId: "workspace",
  rootEvent: id,
  tipEvent: id,
  rootCreatedAt: 10,
  tipCreatedAt: 20,
  enteredStatusAt: 20,
  identifier: `D-${id}`,
  title: `Improve the ${id} experience`,
  body: "Accepted scope",
  status,
  priority: "P1",
  assignee: null,
  relations: [],
  labels: ["ui"],
  externalRefs: [],
});
const record = (payload, at = 1, archived = false) => ({
  event_id: String(at),
  created_at: at,
  record: { record_id: String(at), payload, archived },
});
const context = (issues, extra = {}) => ({
  issues,
  activity: [],
  linkedTasks: [],
  records: [],
  sectionErrors: { tasks: null, comments: null, projects: null },
  snapshotAt: "2026-09-10T12:00:00Z",
  ...extra,
});
test("ready queue excludes dependencies, unfinished attempts and parked imports", () => {
  const a = issue("a"),
    b = issue("b"),
    c = issue("c"),
    d = issue("d");
  a.relations = [{ kind: "blocks", targetIssueId: "b" }];
  const value = context([a, b, c, d], {
    activity: [{ issueId: "d", import: { system: "linear" } }],
    linkedTasks: [{ issueId: "c", status: "assigned" }],
  });
  assert.deepEqual(
    selectAttention(value, "ready", "owner").map((issue) => issue.issueId),
    ["a"],
  );
  value.records = [
    record({ type: "activation", issue_id: "d", active: true }),
    record({ type: "activation", issue_id: "d", active: false }, 2),
  ];
  assert.deepEqual(
    selectAttention(value, "ready", "owner").map((issue) => issue.issueId),
    ["a"],
    "latest explicit parking wins over older activation",
  );
  value.records.push(
    record({ type: "activation", issue_id: "d", active: true }, 3),
  );
  assert.deepEqual(
    selectAttention(value, "ready", "owner").map((issue) => issue.issueId),
    ["a", "d"],
  );
});
test("delegation preserves accountable My Issues visibility and exact decision ownership", () => {
  const a = issue("a");
  const tasks = [
    {
      issueId: "a",
      assigner: "owner",
      assignee: "worker",
      status: "picked_up",
    },
  ];
  assert.equal(selectMyIssues([a], "owner", tasks).length, 1);
  const value = context([a], {
    linkedTasks: tasks,
    records: [
      record({
        type: "decision",
        issue_id: "a",
        decider: "owner",
        selected_option: null,
      }),
    ],
  });
  assert.equal(selectAttention(value, "decision", "owner").length, 1);
  assert.equal(selectAttention(value, "decision", "worker").length, 0);
  assert.equal(selectAttention(value, "delegated", "owner").length, 1);
});
test("capacity advice names its suppression reason and never mutates work", () => {
  const a = issue("a");
  const value = context([a], {
    records: [
      record({
        type: "capacity",
        executor: "worker",
        max_active: 1,
        capabilities: ["ui"],
        paused: false,
      }),
    ],
    linkedTasks: [{ assignee: "worker", status: "blocked" }],
  });
  const snapshot = JSON.stringify(value);
  const [suggestion] = routingSuggestions(value, a);
  assert.equal(suggestion.eligible, false);
  assert.match(suggestion.explanation, /capacity/);
  assert.equal(JSON.stringify(value), snapshot);
});
test("bounded handoff retains current scope and identifies omitted history", () => {
  const a = issue("a");
  a.body = "scope ".repeat(30000);
  const value = context([a], {
    activity: Array.from({ length: 200 }, (_, i) => ({
      issueId: "a",
      createdAt: i,
      reason: "change ".repeat(5000),
    })),
  });
  const text = dispatchContextExport(value, a, 10, 4000);
  assert.ok(text.length <= 4000);
  const parsed = JSON.parse(text);
  assert.equal(parsed.issueId, "a");
  assert.equal(parsed.truncated, true);
  assert.deepEqual(
    relatedIssueSuggestions({ ...a, title: "Repair message delivery" }, [
      { ...issue("b"), title: "Investigate message delivery" },
      issue("unrelated"),
    ]).map((value) => value.issue.issueId),
    ["b"],
  );
});
test("archiving current settings never resurrects an older active setting", () => {
  const values = [
    record({ type: "activation", issue_id: "a", active: true }),
    record({ type: "activation", issue_id: "a", active: false }, 2, true),
  ];
  assert.deepEqual(activeRecords(values, "activation"), []);
});

test("context exports honor their UTF-8 byte budget and preserve valid Unicode JSON", () => {
  const current = issue("unicode");
  current.body = "🛠️完成验证\n".repeat(3000);
  const exported = dispatchContextExport(context([current]), current, 0, 1024);
  assert.ok(new TextEncoder().encode(exported).length <= 1024);
  assert.equal(JSON.parse(exported).truncated, true);
  assert.ok(!exported.includes("\uFFFD"));
});
