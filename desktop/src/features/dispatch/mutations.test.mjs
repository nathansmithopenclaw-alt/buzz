import assert from "node:assert/strict";
import test from "node:test";

import {
  canApplyDispatchMutation,
  publishDispatchComment,
  publishDispatchIssueMutation,
  TrackerMutationConflictError,
} from "./mutations.ts";

const PUBKEY = "a".repeat(64);

function issue(overrides = {}) {
  return {
    streamId: crypto.randomUUID(),
    issueId: crypto.randomUUID(),
    rootEvent: "1".repeat(64),
    rootCreatedAt: 1,
    tipEvent: "2".repeat(64),
    tipCreatedAt: 2,
    revision: 1,
    enteredStatusAt: 1,
    identifier: "DSP-1",
    title: "Mutation fixture",
    body: "",
    status: "todo",
    priority: "P1",
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

function relayEvent(id = "3".repeat(64)) {
  return {
    id,
    pubkey: PUBKEY,
    created_at: 10,
    kind: 40021,
    tags: [],
    content: "{}",
    sig: "4".repeat(128),
  };
}

function context(issues) {
  return {
    issues,
    activity: [],
    comments: [],
    linkedTasks: [],
    projects: [],
    diagnostics: [],
    taskDiagnostics: [],
    viewerRole: "maintainer",
    sectionErrors: { comments: null, projects: null, tasks: null },
    snapshotAt: new Date(0).toISOString(),
  };
}

function dependencies(event = relayEvent()) {
  const calls = { revisions: [], comments: [], published: [] };
  return {
    calls,
    value: {
      identity: async () => ({ pubkey: PUBKEY }),
      buildRevision: async (input) => {
        calls.revisions.push(input);
        return { kind: 40021, content: "{}", createdAt: 10, tags: [] };
      },
      buildComment: async (input) => {
        calls.comments.push(input);
        return { kind: 40022, content: "{}", createdAt: 10, tags: [] };
      },
      sign: async () => event,
      publish: async (published) => {
        calls.published.push(published);
      },
    },
  };
}

test("status mutation round-trips through builders and accepts an absent reason", async () => {
  const current = issue();
  const event = relayEvent();
  const next = {
    ...current,
    status: "in_progress",
    tipEvent: event.id,
    revision: 2,
  };
  const deps = dependencies(event);
  const optimistic = [];
  const rolledBack = [];
  const result = await publishDispatchIssueMutation(
    {
      issue: current,
      mutation: { type: "status", status: "in_progress", reason: null },
      viewerRole: "member",
      onOptimistic: (value) => optimistic.push(value),
      onRollback: (value) => rolledBack.push(value),
      refresh: async () => context([next]),
    },
    deps.value,
  );

  assert.equal(result, next);
  assert.equal(deps.calls.revisions[0].revision.parent_event, current.tipEvent);
  assert.equal(deps.calls.revisions[0].revision.reason, null);
  assert.equal(optimistic[0].tipEvent, event.id);
  assert.equal(rolledBack.length, 0);
  assert.deepEqual(deps.calls.published, [event]);
});

test("a conflicting canonical tip rolls back and refetches", async () => {
  const current = issue();
  const deps = dependencies();
  const rolledBack = [];
  let refreshes = 0;
  await assert.rejects(
    publishDispatchIssueMutation(
      {
        issue: current,
        mutation: { type: "priority", priority: "P0" },
        viewerRole: "member",
        onOptimistic: () => {},
        onRollback: (value) => rolledBack.push(value),
        refresh: async () => {
          refreshes += 1;
          return context([{ ...current, tipEvent: "9".repeat(64) }]);
        },
      },
      deps.value,
    ),
    TrackerMutationConflictError,
  );
  assert.equal(refreshes, 2);
  assert.deepEqual(rolledBack, [current]);
});

test("an applied revision superseded before refresh remains successful", async () => {
  const current = issue();
  const event = relayEvent();
  const superseding = { ...current, tipEvent: "8".repeat(64), revision: 3 };
  const deps = dependencies(event);
  const rolledBack = [];
  let refreshes = 0;
  const result = await publishDispatchIssueMutation(
    {
      issue: current,
      mutation: { type: "priority", priority: "P0" },
      viewerRole: "member",
      onOptimistic: () => {},
      onRollback: (value) => rolledBack.push(value),
      refresh: async () => {
        refreshes += 1;
        return {
          ...context([superseding]),
          activity: [
            {
              issueId: current.issueId,
              eventId: event.id,
            },
          ],
        };
      },
    },
    deps.value,
  );

  assert.equal(result, superseding);
  assert.equal(refreshes, 1);
  assert.deepEqual(rolledBack, []);
});

test("terminal and reopen transitions remain maintainer-only", () => {
  assert.equal(
    canApplyDispatchMutation(
      issue(),
      { type: "status", status: "done", reason: null },
      "member",
    ),
    false,
  );
  assert.equal(
    canApplyDispatchMutation(
      issue({ status: "done" }),
      { type: "status", status: "todo", reason: null },
      "maintainer",
    ),
    true,
  );
});

test("comment mutation publishes the Rust-built signed event and refreshes", async () => {
  const current = issue();
  const event = relayEvent("5".repeat(64));
  const deps = dependencies(event);
  const optimistic = [];
  let refreshed = false;
  const result = await publishDispatchComment(
    {
      body: "  Evidence added.  ",
      issue: current,
      onOptimistic: (value) => optimistic.push(value),
      onRollback: () => assert.fail("comment should not roll back"),
      refresh: async () => {
        refreshed = true;
        return context([current]);
      },
    },
    deps.value,
  );

  assert.equal(result, event.id);
  assert.equal(deps.calls.comments[0].comment.body, "Evidence added.");
  assert.equal(optimistic[0].eventId, event.id);
  assert.equal(refreshed, true);
});
