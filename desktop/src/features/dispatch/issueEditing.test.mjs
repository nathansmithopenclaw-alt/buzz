import assert from "node:assert/strict";
import test from "node:test";
import {
  dueDateInput,
  dueFromDateInput,
  editRevisionFields,
  issueAlias,
} from "./issueEditing.ts";

test("one edit builds sparse scalar, relation and reference changes without clearing untouched fields", () => {
  const root = "a".repeat(64);
  const oldRelation = {
    kind: "blocks",
    targetIssueId: "old",
    targetRootEvent: root,
  };
  const newRelation = {
    kind: "parent",
    targetIssueId: "new",
    targetRootEvent: root,
  };
  const task = { type: "task", ref: "task-id" };
  const issue = {
    title: "Old",
    labels: [],
    projectId: "project",
    relations: [oldRelation],
    externalRefs: [task],
  };
  const result = editRevisionFields(issue, {
    title: " New ",
    projectId: null,
    labels: [" UI ", "ui"],
    relations: [newRelation],
    externalRefs: [task, { type: "url", ref: "https://example.test" }],
  });
  assert.deepEqual(result.set, { title: "New", labels: ["ui"] });
  assert.deepEqual(result.clear, ["project_id"]);
  assert.deepEqual(result.relations.add, [
    { type: "parent", target_issue_id: "new", target_root_event: root },
  ]);
  assert.deepEqual(result.relations.remove, [
    { type: "blocks", target_issue_id: "old", target_root_event: root },
  ]);
  assert.deepEqual(result.external_refs.remove, []);
  assert.equal(result.external_refs.add.length, 1);
});

test("calendar due dates round trip and reject impossible dates", () => {
  for (const value of ["2026-03-08", "2026-11-01", "2026-09-10"])
    assert.equal(dueDateInput(dueFromDateInput(value)), value);
  assert.throws(() => dueFromDateInput("2026-02-30"), /valid due date/);
  assert.equal(dueFromDateInput(""), null);
});

test("human aliases grow when a short prefix would identify more than one issue", () => {
  const first = { issueId: "one", identifier: `D-${"a".repeat(32)}` };
  const second = { issueId: "two", identifier: `D-aaaaaabb${"b".repeat(24)}` };
  assert.equal(issueAlias(first, [first, second]), "D-AAAAAAAA");
  assert.equal(issueAlias(second, [first, second]), "D-AAAAAABB");
  assert.equal(
    issueAlias({ issueId: "three", identifier: "BUG-42" }),
    "BUG-42",
  );
});

test("reconciliation applies relationship and reference deltas without dropping untouched links", async () => {
  const { proposedConflictFields } = await import("./conflicts.ts");
  const current = {
    relations: [
      { kind: "blocks", targetIssueId: "a", targetRootEvent: "root-a" },
    ],
    externalRefs: [
      { type: "task", ref: "accepted-attempt" },
      { type: "url", ref: "old" },
    ],
  };
  const change = proposedConflictFields(
    {
      proposed: {
        set: { title: "Proposed" },
        clear: [],
        relations: {
          add: [
            {
              type: "parent",
              target_issue_id: "b",
              target_root_event: "root-b",
            },
          ],
          remove: [
            {
              type: "blocks",
              target_issue_id: "a",
              target_root_event: "root-a",
            },
          ],
        },
        external_refs: {
          add: [{ type: "url", ref: "new" }],
          remove: [{ type: "url", ref: "old" }],
        },
      },
    },
    current,
  );
  assert.deepEqual(change.relations, [
    { kind: "parent", targetIssueId: "b", targetRootEvent: "root-b" },
  ]);
  assert.deepEqual(change.externalRefs, [
    { type: "task", ref: "accepted-attempt" },
    { type: "url", ref: "new" },
  ]);
  assert.equal(current.externalRefs[1].ref, "old");
});
