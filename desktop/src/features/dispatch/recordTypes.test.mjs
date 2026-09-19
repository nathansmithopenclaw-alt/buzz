import test from "node:test";
import assert from "node:assert/strict";
import { activeRecords } from "./recordTypes.ts";

const settings = [
  {
    type: "capture",
    channel: "channel-a",
    workspace: "workspace-a",
    project: null,
    enabled: true,
  },
  {
    type: "unowned_alert",
    workspace: "workspace-a",
    channel: null,
    after_hours: 24,
    enabled: true,
  },
  {
    type: "autopilot",
    workspace: "workspace-a",
    project: "project-a",
    agents: ["agent"],
    label: null,
    at_a_time: 2,
    per_day: 10,
    reviewer_required: true,
    enabled: true,
    activation_id: "activation",
    confirmation_required: false,
    confirmation_event: null,
  },
];

function projection(payload, id, created_at, archived = false) {
  return {
    event_id: id,
    created_at,
    record: { record_id: id, payload, archived },
  };
}

for (const payload of settings) {
  test(`${payload.type}: newer record ID wins the same authorization slot`, () => {
    const older = projection(payload, "f".repeat(64), 1);
    const newer = projection({ ...payload, enabled: false }, "a".repeat(64), 2);
    for (const values of [
      [older, newer],
      [newer, older],
    ]) {
      assert.deepEqual(activeRecords(values, payload.type), [newer]);
    }
  });

  test(`${payload.type}: event ID breaks timestamp ties independent of input order`, () => {
    const lower = projection(payload, "a".repeat(64), 2);
    const higher = projection(
      { ...payload, enabled: false },
      "f".repeat(64),
      2,
    );
    for (const values of [
      [lower, higher],
      [higher, lower],
    ]) {
      assert.deepEqual(activeRecords(values, payload.type), [higher]);
    }
  });

  test(`${payload.type}: archiving the winning record does not resurrect its predecessor`, () => {
    const older = projection(payload, "a".repeat(64), 1);
    const archived = projection(payload, "b".repeat(64), 2, true);
    assert.deepEqual(activeRecords([older, archived], payload.type), []);
  });
}

test("authorization singleton keys match relay channel, workspace and project scopes", () => {
  const [capture, alert, autopilot] = settings;
  const scopes = [
    [
      capture,
      { ...capture, workspace: "workspace-b" },
      { ...capture, channel: "channel-b" },
    ],
    [
      alert,
      { ...alert, channel: "channel-b" },
      { ...alert, workspace: "workspace-b" },
    ],
    [
      autopilot,
      { ...autopilot, activation_id: "next" },
      { ...autopilot, workspace: "workspace-b" },
      { ...autopilot, project: "project-b" },
    ],
  ];
  for (const [original, replacement, ...otherSlots] of scopes) {
    const older = projection(original, "old", 1);
    const newer = projection(replacement, "new", 2);
    const others = otherSlots.map((payload, i) =>
      projection(payload, `other-${i}`, 3),
    );
    assert.deepEqual(activeRecords([older, newer, ...others], original.type), [
      newer,
      ...others,
    ]);
  }
});
