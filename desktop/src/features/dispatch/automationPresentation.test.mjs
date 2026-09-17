import test from "node:test";
import assert from "node:assert/strict";
import {
  automationActivity,
  automationState,
  initialAutomation,
} from "./automationPresentation.ts";

const controls = () => ({
  ...initialAutomation("workspace"),
  service_id: "worker",
  service_pubkey: "service",
  capture_channels: ["channel"],
});
const record = () => ({
  event_id: "settings",
  record: {
    record_id: "config",
    archived: false,
    payload: { type: "automation", controls: controls() },
  },
});
function fixture() {
  const settings = record();
  return {
    records: [settings],
    issues: [],
    activity: [],
    sectionErrors: {},
    admission: { unsealed: 0, diagnostics: [] },
    automation: {
      actions: [],
      heartbeats: [
        {
          signer: "service",
          serviceId: "worker",
          observedAt: 1000,
          health: {
            status: "healthy",
            detail: JSON.stringify({
              approval: "settings",
              controls: "preview",
              spent: 0,
              pending: 0,
            }),
          },
        },
      ],
    },
  };
}
test("turn-on requires a fresh matching response, confirmed budget and complete permissions", () => {
  const context = fixture();
  assert.equal(
    automationState(context, context.records[0], 1000).canStart,
    true,
  );
  const mutations = [
    (c) => {
      c.automation.heartbeats[0].signer = "outsider";
    },
    (c) => {
      c.automation.heartbeats[0].observedAt = 600;
    },
    (c) => {
      c.automation.heartbeats[0].observedAt = 1400;
    },
    (c) => {
      c.automation.heartbeats[0].health.status = "degraded";
    },
    (c) => {
      c.automation.heartbeats[0].health.detail = JSON.stringify({
        approval: "old-settings",
        controls: "preview",
        spent: 0,
        pending: 0,
      });
    },
    (c) => {
      c.automation.heartbeats[0].health.detail = JSON.stringify({
        approval: "settings",
        controls: "active",
        spent: 0,
        pending: 0,
      });
    },
    (c) => {
      c.automation.heartbeats[0].health.detail = JSON.stringify({
        approval: "settings",
        controls: "preview",
        pending: 0,
      });
    },
    (c) => {
      c.automation.heartbeats[0].health.detail = JSON.stringify({
        approval: "settings",
        controls: "preview",
        spent: 1,
        pending: 1,
      });
    },
    (c) => {
      c.automation.heartbeats[0].health.detail = JSON.stringify({
        approval: "settings",
        controls: "preview",
        spent: -1,
        pending: 0,
      });
    },
    (c) => {
      c.admission.unsealed = 1;
    },
    (c) => {
      c.sectionErrors.records = "offline";
    },
    (c) => {
      c.records[0].record.payload.controls.capture_channels = [];
    },
  ];
  for (const mutate of mutations) {
    const changed = fixture();
    mutate(changed);
    assert.equal(
      automationState(changed, changed.records[0], 1000).canStart,
      false,
      mutate.toString(),
    );
  }
});
test("pausing, expired runs and used limits are explained without claiming completion", () => {
  const context = fixture();
  const value = context.records[0];
  value.record.payload.controls.mode = "paused";
  assert.equal(automationState(context, value, 1000).label, "Pausing…");
  context.automation.heartbeats[0].health.detail = JSON.stringify({
    approval: "settings",
    controls: "paused",
    spent: 3,
    pending: 0,
  });
  assert.equal(automationState(context, value, 1000).label, "Paused");
  assert.equal(automationState(context, value, 1000).remaining, 0);
  value.record.payload.controls.mode = "preview";
  context.automation.heartbeats[0].health.detail = JSON.stringify({
    approval: "settings",
    controls: "preview",
    spent: 3,
    pending: 0,
  });
  assert.equal(automationState(context, value, 1000).label, "Limit reached");
  assert.equal(automationState(context, value, 1000).canStart, false);
  value.record.payload.controls.mode = "active";
  value.record.payload.controls.expires_at = 999;
  context.automation.heartbeats[0].health.detail = JSON.stringify({
    approval: "settings",
    controls: "active",
    spent: 1,
    pending: 0,
  });
  assert.equal(automationState(context, value, 1000).label, "Time is up");
});
test("activity includes accepted message captures, but not another author or a later edit", () => {
  const context = fixture();
  context.issues = [
    {
      issueId: "captured",
      rootEvent: "root",
      sourceEventId: "message",
      title: "Captured work",
    },
    {
      issueId: "human",
      rootEvent: "human-root",
      sourceEventId: "message-2",
      title: "Human work",
    },
    {
      issueId: "edited",
      rootEvent: "original",
      sourceEventId: "message-3",
      title: "Only edited by service",
    },
  ];
  context.activity = [
    {
      issueId: "captured",
      eventId: "root",
      revision: 0,
      authorPubkey: "service",
      createdAt: 999,
      import: null,
    },
    {
      issueId: "human",
      eventId: "human-root",
      revision: 0,
      authorPubkey: "human",
      createdAt: 999,
      import: null,
    },
    {
      issueId: "edited",
      eventId: "edit",
      revision: 1,
      authorPubkey: "service",
      createdAt: 999,
      import: null,
    },
  ];
  context.automation.actions = [
    {
      eventId: "assign",
      issueId: "captured",
      signer: "service",
      createdAt: 1000,
      action: "assign",
      result: "Accepted assignment",
    },
  ];
  const history = automationActivity(context);
  assert.deepEqual(
    history.map((event) => event.result),
    ["Sent to an agent", "Created from a message"],
  );
  assert.equal(history[1].issueId, "captured");
  assert.equal(
    history.some((event) => /complete|verified/i.test(event.result)),
    false,
  );
  context.activity[0].import = { system: "archive" };
  assert.equal(automationActivity(context).length, 1);
});

test("connection restrictions explain the user's next step without internal terminology", () => {
  for (const [diagnostic, message] of [
    [
      "Approval capture channels exceed the host allowlist",
      "cannot watch one of the channels",
    ],
    [
      "Approval notice channel differs from the host decision surface",
      "cannot post reminders",
    ],
    ["Approval routing exceeds the host allowlist", "cannot send work"],
  ]) {
    const context = fixture();
    context.automation.heartbeats[0].health = {
      status: "degraded",
      detail: diagnostic,
    };
    const state = automationState(context, context.records[0], 1000);
    assert.equal(state.canStart, false);
    assert.ok(state.explanation.includes(message));
    assert.doesNotMatch(
      state.explanation,
      /allowlist|approval|host decision surface/,
    );
  }
});
