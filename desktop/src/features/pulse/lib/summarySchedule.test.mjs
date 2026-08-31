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
  assert.match(yaml, /@Nectar Write today's Summary/);
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
  assert.match(yaml, /@EvilName Write today's Summary/);
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
