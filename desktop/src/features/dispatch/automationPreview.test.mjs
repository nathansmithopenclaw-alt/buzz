import test from "node:test";
import assert from "node:assert/strict";
import { automationPreview } from "./automationPreview.ts";
const issue = (id) => ({
  issueId: id,
  title: id,
  rootEvent: id,
  status: "todo",
  assignee: "owner",
  labels: ["testing"],
  relations: [],
  externalRefs: [],
});
const controls = {
  issue_ids: ["a", "b"],
  routing: [{ label: "testing", executor: "worker", channel: "channel" }],
  notify_triage: false,
  unassigned_after_secs: null,
};
const context = () => ({
  issues: [issue("a"), issue("b")],
  linkedTasks: [],
  activity: [],
  sectionErrors: {},
  records: [
    {
      event_id: "capacity",
      created_at: 1,
      record: {
        record_id: "capacity",
        archived: false,
        payload: {
          type: "capacity",
          executor: "worker",
          max_active: 1,
          capabilities: ["testing"],
          paused: false,
        },
      },
    },
  ],
});
test("one remaining executor slot cannot be offered to two candidates", () => {
  const values = automationPreview(context(), controls);
  assert.equal(values[0].ready, true);
  assert.equal(values[1].ready, false);
  assert.match(values[1].reason, /use this agent’s available room/);
});
test("review, missing linked history and an incomplete source each hold routing", () => {
  const value = context();
  value.linkedTasks = [
    { taskId: "attempt", issueId: "a", status: "done", assignee: "worker" },
  ];
  assert.match(
    automationPreview(value, controls)[0].reason,
    /waiting for review/,
  );
  value.linkedTasks = [];
  value.issues[0].externalRefs = [{ type: "task", ref: "attempt" }];
  assert.match(
    automationPreview(value, controls)[0].reason,
    /needs the linked task’s latest update/,
  );
  value.sectionErrors.tasks = "Offline";
  assert.match(
    automationPreview(value, controls)[0].reason,
    /Refresh Dispatch/,
  );
});
test("parked imports and self-review cannot be offered as eligible", () => {
  const value = context();
  value.activity = [{ issueId: "a", import: { system: "linear" } }];
  value.issues[1].assignee = "worker";
  const preview = automationPreview(value, controls);
  assert.match(preview[0].reason, /enable it for current work first/);
  assert.match(preview[1].reason, /someone other than the agent/);
});
