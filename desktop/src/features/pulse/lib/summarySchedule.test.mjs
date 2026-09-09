import assert from "node:assert/strict";
import test from "node:test";

import {
  SUMMARY_WORKFLOW_NAME,
  findSummaryWorkflow,
  summaryWorkflowYaml,
} from "./summarySchedule.ts";

test("summary workflow yaml carries local cron and timezone", () => {
  const yaml = summaryWorkflowYaml({
    hour: 9,
    minute: 0,
    timezone: "America/New_York",
    reporterName: "Nectar",
  });
  assert.match(yaml, new RegExp(`name: ${SUMMARY_WORKFLOW_NAME}`));
  assert.match(yaml, /on: schedule/);
  assert.match(yaml, /cron: '0 9 \* \* \*'/);
  assert.match(yaml, /timezone: America\/New_York/);
  assert.match(yaml, /@Nectar Write a Summary/);
});

test("summary workflow yaml sanitizes reporter names and clamps time", () => {
  const yaml = summaryWorkflowYaml({
    hour: 30,
    minute: -4,
    timezone: "America/New_York\nUTC",
    reporterName: "@Evil\nName",
  });
  assert.match(yaml, /cron: '0 23 \* \* \*'/);
  assert.match(yaml, /timezone: America\/New_YorkUTC/);
  assert.match(yaml, /@EvilName Write a Summary/);
  assert.doesNotMatch(yaml, /@Evil\\n/);
});

test("findSummaryWorkflow matches name and owner", () => {
  const found = findSummaryWorkflow(
    [
      {
        id: "other",
        revision: "r",
        name: "Standup",
        ownerPubkey: "aa",
        channelId: "c1",
        definition: {},
        status: "active",
        createdAt: 1,
        updatedAt: 1,
      },
      {
        id: "mine",
        revision: "r",
        name: "Summary",
        ownerPubkey: "AA",
        channelId: "c1",
        definition: {},
        status: "active",
        createdAt: 1,
        updatedAt: 1,
      },
    ],
    "aa",
  );
  assert.equal(found?.id, "mine");
});

test("Summary supports weekly, monthly, multiple hours and recurring intervals without changing the prompt", () => {
  for (const schedule of [
    { on: "schedule", cron: "30 17 * * 1,3,5" },
    { on: "schedule", cron: "0 9 15 * *" },
    { on: "schedule", cron: "0 9,13,17 * * *" },
    { on: "schedule", interval: "30m" },
  ]) {
    const yaml = summaryWorkflowYaml({
      hour: 9,
      minute: 0,
      timezone: "America/New_York",
      reporterName: "Nectar",
      ownerPubkey: "owner",
      schedule,
    });
    assert.ok(yaml.includes(schedule.cron ?? schedule.interval));
    assert.match(yaml, /timezone: America\/New_York/);
    assert.match(yaml, /since your previous Summary/);
    assert.doesNotMatch(yaml, /today's|daily Summary/);
  }
});
test("invalid schedules fail before workflow generation", () => {
  for (const schedule of [
    { on: "schedule", cron: "nonsense" },
    { on: "schedule", interval: "0m" },
    { on: "schedule", cron: "0 9 * * *", interval: "1h" },
  ]) {
    assert.throws(() =>
      summaryWorkflowYaml({
        hour: 9,
        minute: 0,
        timezone: "UTC",
        reporterName: "Nectar",
        schedule,
      }),
    );
  }
});
