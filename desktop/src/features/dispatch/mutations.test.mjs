import assert from "node:assert/strict";
import test from "node:test";

import {
  canApplyDispatchMutation,
  publishDispatchComment,
  publishDispatchIssueCreate,
  publishDispatchIssueMutation,
  TRACKER_REQUEST_MATCH_KINDS,
  TrackerMutationConflictError,
  TrackerRevisionRejectedError,
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

function createInput(overrides = {}) {
  const requestId = crypto.randomUUID();
  return {
    streamId: crypto.randomUUID(),
    requestId,
    identifier: `DSP-${requestId.replaceAll("-", "")}`,
    title: "Desktop-created issue",
    body: "  Body\n",
    status: "triage",
    priority: "P1",
    projectId: null,
    labels: [" UI ", "desktop", "ui"],
    assignee: PUBKEY,
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

test("status mutation requires a non-empty reason before building", async () => {
  const current = issue();
  const deps = dependencies();
  const optimistic = [];
  await assert.rejects(
    publishDispatchIssueMutation(
      {
        issue: current,
        mutation: { type: "status", status: "in_progress", reason: "   " },
        viewerRole: "member",
        onOptimistic: (value) => optimistic.push(value),
        onRollback: () => assert.fail("must not roll back a blocked emit"),
        refresh: async () => assert.fail("must not refresh a blocked emit"),
      },
      deps.value,
    ),
    /status change reason is required/,
  );
  assert.equal(deps.calls.revisions.length, 0);
  assert.equal(deps.calls.published.length, 0);
  assert.equal(optimistic.length, 0);
});

test("status mutation round-trips a required reason through builders", async () => {
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
      mutation: {
        type: "status",
        status: "in_progress",
        reason: " Execution started ",
      },
      viewerRole: "member",
      onOptimistic: (value) => optimistic.push(value),
      onRollback: (value) => rolledBack.push(value),
      refresh: async () => context([next]),
    },
    deps.value,
  );

  assert.equal(result, next);
  assert.equal(deps.calls.revisions[0].revision.parent_event, current.tipEvent);
  assert.equal(deps.calls.revisions[0].revision.reason, "Execution started");
  assert.equal(optimistic[0].tipEvent, event.id);
  assert.equal(rolledBack.length, 0);
  assert.deepEqual(deps.calls.published, [event]);
});

test("a fold-rejected status revision rolls back instead of keeping optimistic state", async () => {
  const current = issue();
  const event = relayEvent();
  const deps = dependencies(event);
  const optimistic = [];
  const rolledBack = [];
  await assert.rejects(
    publishDispatchIssueMutation(
      {
        issue: current,
        mutation: {
          type: "status",
          status: "in_progress",
          reason: "Execution started",
        },
        viewerRole: "member",
        onOptimistic: (value) => optimistic.push(value),
        onRollback: (value) => rolledBack.push(value),
        refresh: async () => ({
          ...context([current]),
          diagnostics: [
            {
              kind: "InvalidEvent",
              message: `InvalidEvent { event_id: "${event.id}", reason: "status change reason must be non-empty when present" }`,
            },
          ],
        }),
      },
      deps.value,
    ),
    TrackerRevisionRejectedError,
  );
  assert.equal(optimistic[0].status, "in_progress");
  assert.deepEqual(rolledBack, [current]);
  assert.deepEqual(deps.calls.published, [event]);
});

test("issue create uses the Rust root builder and refreshes revision zero", async () => {
  const input = createInput();
  const event = {
    ...relayEvent("6".repeat(64)),
    kind: 40020,
    tags: [["request", input.requestId]],
  };
  const calls = { roots: [], published: [] };
  const created = issue({
    streamId: input.streamId,
    issueId: input.requestId,
    rootEvent: event.id,
    tipEvent: event.id,
    revision: 0,
    status: "triage",
  });
  const result = await publishDispatchIssueCreate(
    { input, refresh: async () => context([created]) },
    {
      identity: async () => ({ pubkey: PUBKEY }),
      buildIssue: async (value) => {
        calls.roots.push(value);
        return { kind: 40020, content: "{}", createdAt: 10, tags: event.tags };
      },
      sign: async () => event,
      fetchRequestEvents: async () => [],
      publish: async (value) => calls.published.push(value),
    },
  );

  assert.equal(result.issue.revision, 0);
  assert.equal(result.replayed, false);
  assert.equal(calls.roots[0].root.issue_id, input.requestId);
  assert.equal(calls.roots[0].root.body, input.body);
  assert.deepEqual(calls.roots[0].root.labels, ["desktop", "ui"]);
  assert.deepEqual(calls.published, [event]);
});

test("issue create replays the same literal request tag without republishing", async () => {
  const input = createInput();
  const event = {
    ...relayEvent("7".repeat(64)),
    kind: 40020,
    tags: [["request", input.requestId]],
  };
  const created = issue({
    streamId: input.streamId,
    issueId: input.requestId,
    rootEvent: event.id,
    tipEvent: event.id,
    revision: 0,
  });
  let published = false;
  const result = await publishDispatchIssueCreate(
    { input, refresh: async () => context([created]) },
    {
      identity: async () => ({ pubkey: PUBKEY }),
      buildIssue: async () => ({
        kind: 40020,
        content: event.content,
        createdAt: 11,
        tags: event.tags,
      }),
      sign: async () => ({ ...event, created_at: 11 }),
      fetchRequestEvents: async () => [event],
      publish: async () => {
        published = true;
      },
    },
  );

  assert.equal(result.replayed, true);
  assert.equal(published, false);
});

test("issue-create request matching still fetches kind-40025 project activity", () => {
  assert.deepEqual(
    TRACKER_REQUEST_MATCH_KINDS,
    [40020, 40021, 40022, 40025, 30621],
  );
});

test("issue create rejects a request ID already used by another Tracker kind", async () => {
  const input = createInput();
  const candidate = {
    ...relayEvent("8".repeat(64)),
    kind: 40020,
    tags: [["request", input.requestId]],
  };
  const existingRevision = {
    ...relayEvent("9".repeat(64)),
    tags: [["request", input.requestId]],
  };

  await assert.rejects(
    publishDispatchIssueCreate(
      { input, refresh: async () => context([]) },
      {
        identity: async () => ({ pubkey: PUBKEY }),
        buildIssue: async () => ({
          kind: 40020,
          content: candidate.content,
          createdAt: 11,
          tags: candidate.tags,
        }),
        sign: async () => candidate,
        fetchRequestEvents: async () => [existingRevision],
        publish: async () => assert.fail("must not publish"),
      },
    ),
    /different Tracker input/,
  );
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
      { type: "status", status: "done", reason: "Closing" },
      "member",
    ),
    false,
  );
  assert.equal(
    canApplyDispatchMutation(
      issue({ status: "done" }),
      { type: "status", status: "todo", reason: "Reopening" },
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
