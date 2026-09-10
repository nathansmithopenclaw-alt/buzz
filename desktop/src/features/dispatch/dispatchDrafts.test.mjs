import {
  acknowledgeIssueWrite,
  publishRetainedIssueWrite,
} from "./issueWriteRecovery.ts";
import test from "node:test";
import assert from "node:assert/strict";
import { dispatchDrafts, keepDispatchDraft } from "./dispatchDrafts.ts";
import { assignDispatchIssue } from "./taskAssignments.ts";

function fixture() {
  const values = new Map();
  const environment = {
    relay: "wss://original.test",
    signer: "a".repeat(64),
    failedRead: false,
    failedWrite: false,
    afterWrite: null,
  };
  const target = new EventTarget();
  target.localStorage = {
    getItem: (key) => {
      if (environment.failedRead) throw new Error("denied");
      return values.get(key) ?? null;
    },
    setItem: (key, value) => {
      if (environment.failedWrite) throw new Error("quota");
      values.set(key, value);
      environment.afterWrite?.();
    },
  };
  target.__TAURI_INTERNALS__ = {
    invoke: async (command) => {
      if (command === "get_relay_ws_url") return environment.relay;
      if (command === "get_identity")
        return { pubkey: environment.signer, display_name: "Reviewer" };
      throw new Error(`Unexpected native operation: ${command}`);
    },
  };
  globalThis.window = target;
  const value = {
    relay: environment.relay,
    signer: environment.signer,
    workspace: crypto.randomUUID(),
    category: "Issue edit",
    issueId: crypto.randomUUID(),
    event: {
      id: "b".repeat(64),
      kind: 40021,
      pubkey: environment.signer,
      tags: [],
      content: '{"set":{"title":"Keep this work"}}',
      sig: "c".repeat(128),
      created_at: 100,
    },
  };
  return { values, environment, value };
}
test("archive-before-clear survives interruption and retry without duplicating or changing signed bytes", async () => {
  const { value } = fixture();
  await assert.rejects(
    keepDispatchDraft(value, () => {
      throw new Error("interrupted clearing pending");
    }),
    /interrupted/,
  );
  const saved = await dispatchDrafts(value.workspace);
  assert.equal(saved.length, 1);
  assert.deepEqual(saved[0].event, value.event);
  let cleared = 0;
  await keepDispatchDraft(value, () => cleared++);
  assert.equal((await dispatchDrafts(value.workspace)).length, 1);
  assert.equal(cleared, 1);
});
test("failed storage and a community switch never discard the pending change", async () => {
  const { environment, value } = fixture();
  let cleared = 0;
  environment.failedWrite = true;
  await assert.rejects(
    keepDispatchDraft(value, () => cleared++),
    /could not be saved/,
  );
  assert.equal(cleared, 0);
  environment.failedWrite = false;
  environment.afterWrite = () => {
    environment.relay = "wss://other.test";
  };
  await assert.rejects(
    keepDispatchDraft(value, () => cleared++),
    /original community/,
  );
  assert.equal(cleared, 0);
  environment.relay = value.relay;
  assert.equal((await dispatchDrafts(value.workspace)).length, 1);
});
test("unreadable local storage is never treated as an empty journal", async () => {
  const { environment, value } = fixture();
  environment.failedRead = true;
  await assert.rejects(dispatchDrafts(value.workspace), /cannot be read/);
  await assert.rejects(
    keepDispatchDraft(value, () => assert.fail("must not clear")),
    /cannot be read/,
  );
});
test("concurrent recovery of the same event retains one draft", async () => {
  const { value } = fixture();
  await Promise.all([
    keepDispatchDraft(value, () => {}),
    keepDispatchDraft(value, () => {}),
  ]);
  assert.equal((await dispatchDrafts(value.workspace)).length, 1);
});
test("assignment operation rejects unreviewed routing conditions and unresolved prior execution", async () => {
  const { value } = fixture();
  const issue = {
    streamId: value.workspace,
    issueId: value.issueId,
    tipEvent: "tip",
    title: "Scope",
    status: "todo",
    labels: [],
    externalRefs: [],
    relations: [],
  };
  const context = {
    issues: [issue],
    linkedTasks: [],
    records: [],
    activity: [],
    sectionErrors: {},
    viewerRole: "maintainer",
  };
  const input = {
    issue,
    executor: "d".repeat(64),
    reviewer: value.signer,
    channel: issue.streamId,
    method: "Review artifact",
    overrideReason: "",
    refresh: async () => context,
  };
  await assert.rejects(assignDispatchIssue(input), /No configured capacity/);
  issue.externalRefs.push({ type: "task_event", ref: "e".repeat(64) });
  await assert.rejects(
    assignDispatchIssue({
      ...input,
      overrideReason: "Explicit controlled attempt",
    }),
    /no accepted task history/,
  );
});

test("a scope switch cannot publish or clear a matching pending event in the other community", async () => {
  const { environment, value, values } = fixture();
  values.set("original journal", "retain original");
  values.set("other journal", "retain other");
  const before = new Map(values);
  environment.relay = "wss://other.test";
  await assert.rejects(
    acknowledgeIssueWrite(value.event, value.relay),
    /original community/,
  );
  await assert.rejects(
    publishRetainedIssueWrite(value.event, "timeout", "failed", value.relay),
    /original community/,
  );
  assert.deepEqual(values, before);
});
