import { expect, test, type Page } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";
import {
  AGENT,
  OWNER,
  WORKSPACE,
  automationFixture,
  planningRecord,
  publishedRecords,
} from "../helpers/automationFixture";
import type { AutopilotAuthorization } from "../../src/features/dispatch/recordTypes";
// Activation can create capacity and authorization; each confirms paced history reads.
const PROJECT = "31334708-f5ba-4d7e-b7d6-8bd63984de30";
const ACTIVATION = "624f136a-e4e9-4af6-942e-d722ca9bc1e0";
const RELAY = "e".repeat(64);
const THIRD = "3".repeat(64);
function payload(): AutopilotAuthorization {
  return {
    type: "autopilot",
    workspace: WORKSPACE,
    project: PROJECT,
    agents: [AGENT],
    label: null,
    at_a_time: 2,
    per_day: 10,
    reviewer_required: true,
    enabled: true,
    activation_id: ACTIVATION,
    confirmation_required: true,
    confirmation_event: null,
  };
}
function fixture(on = false, confirmation = false) {
  const context = automationFixture();
  context.canAuthorizeAutomation = true;
  context.records = on ? [planningRecord(payload())] : [];
  context.projects = [
    { projectId: PROJECT, name: "Website launch", slug: "website-launch" },
  ];
  context.issues[0].projectId = PROJECT;
  context.automation = {
    heartbeats: [],
    relaySigner: RELAY,
    actions: [],
    outcomes: confirmation
      ? [
          {
            eventId: "4".repeat(64),
            requestKey: `confirmation:${ACTIVATION}`,
            source: "a".repeat(64),
            result:
              "Autopilot made 3 assignments in the first hour. Keep it on?",
            confirmationRequired: true,
            confirmationEvent: THIRD,
          },
        ]
      : [],
  };
  return context;
}
async function open(page: Page, context = fixture(), project = true) {
  await installMockBridge(page, {
    dispatch: { workspace: { id: WORKSPACE, name: "Product work" }, context },
    searchProfiles: [
      { pubkey: OWNER, displayName: "Tyler" },
      { pubkey: AGENT, displayName: "Research helper", isAgent: true },
    ],
  });
  await page.goto("/");
  await page.getByTestId("open-dispatch-view").click();
  await expect(page.getByTestId("dispatch-new-issue")).toBeEnabled();
  if (project) {
    await page
      .getByRole("navigation", { name: "Dispatch views" })
      .getByRole("button", { name: "Projects", exact: true })
      .click();
    await page.getByRole("button", { name: /Website launch/ }).click();
    await expect(page.getByTestId("dispatch-autopilot")).toBeVisible();
  }
}
async function chooseAgent(page: Page) {
  await page
    .getByRole("button", { name: "Choose an agent", exact: true })
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: /Research helper/ })
    .click();
}
async function workflows(page: Page) {
  return page.evaluate(
    async (channelId) =>
      await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("get_channel_workflows", {
        channelId,
      }),
    WORKSPACE,
  ) as Promise<Array<{ id: string; definition: Record<string, unknown> }>>;
}
test("card defaults and review line derive from chosen values; ON writes approval and schedule", async ({
  page,
}, info) => {
  await open(page);
  const card = page.getByTestId("dispatch-autopilot");
  await expect(card.getByLabel("At a time", { exact: true })).toHaveValue("2");
  await expect(card.getByLabel("Per day")).toHaveValue("10");
  await expect(
    card.getByRole("switch", { name: "Reviewer required" }),
  ).toBeChecked({ timeout: 15000 });
  await expect(
    card.getByRole("switch", { name: "Autopilot", exact: true }),
  ).not.toBeChecked();
  expect(await publishedRecords(page)).toHaveLength(0);
  await waitForAnimations(page);
  await card.screenshot({ path: info.outputPath("autopilot-default.png") });
  await chooseAgent(page);
  await expect(card.getByLabel("Issues at the same time")).toHaveValue("2");
  await card.getByLabel("At a time", { exact: true }).fill("4");
  await card.getByLabel("Per day").fill("12");
  await card.getByLabel("Only issues labeled", { exact: true }).check();
  await card.getByLabel("Issue label", { exact: true }).fill("research");
  await expect(card.getByTestId("autopilot-review-line")).toHaveText(
    "When an issue in this project is in Todo with no blockers and labeled research, Buzz assigns it to Research helper, at most 4 at a time and 12 per day. The issue owner reviews the result.",
  );
  await card.getByRole("switch", { name: "Autopilot", exact: true }).click();
  await expect(
    card.getByRole("switch", { name: "Autopilot", exact: true }),
  ).toBeChecked({ timeout: 15000 });
  const records = await publishedRecords(page);
  expect(records.find((r) => r.payload.type === "capacity").payload).toEqual({
    type: "capacity",
    executor: AGENT,
    max_active: 2,
    capabilities: [],
    paused: false,
  });
  const approval = records.find((r) => r.payload.type === "autopilot");
  expect(approval).toMatchObject({
    v: 1,
    revision: 0,
    parent_event: null,
    archived: false,
    payload: {
      ...payload(),
      activation_id: expect.any(String),
      at_a_time: 4,
      per_day: 12,
      label: "research",
    },
  });
  expect(
    (await workflows(page)).find(
      (w) => w.definition.name === `dispatch-autopilot-${PROJECT}`,
    )?.definition,
  ).toEqual({
    name: `dispatch-autopilot-${PROJECT}`,
    enabled: true,
    trigger: { on: "schedule", interval: "1m" },
    steps: [{ id: "assign", action: "dispatch.assign" }],
  });
  await waitForAnimations(page);
  await card.screenshot({ path: info.outputPath("autopilot-on-review.png") });
});
test("OFF disables the same workflow and revises the same record without deleting history", async ({
  page,
}) => {
  await open(page);
  await chooseAgent(page);
  const toggle = page.getByRole("switch", { name: "Autopilot", exact: true });
  await toggle.click();
  await expect(toggle).toBeChecked({ timeout: 15000 });
  const first = (await publishedRecords(page)).find(
    (r) => r.payload.type === "autopilot",
  );
  const workflow = (await workflows(page)).find(
    (w) => w.definition.name === `dispatch-autopilot-${PROJECT}`,
  );
  await toggle.click();
  await expect(toggle).not.toBeChecked();
  await expect
    .poll(
      async () =>
        (await workflows(page)).find((w) => w.id === workflow?.id)?.definition
          .enabled,
    )
    .toBe(false);
  const last = (await publishedRecords(page)).at(-1);
  expect(last).toMatchObject({
    record_id: first.record_id,
    revision: 1,
    archived: false,
    payload: { enabled: false, activation_id: first.payload.activation_id },
  });
  expect(last.parent_event).toMatch(/^[a-f0-9]{64}$/);
  expect(last.request_id).not.toBe(first.request_id);
  expect(
    await page.evaluate(() =>
      window.__BUZZ_E2E_COMMANDS__?.includes("delete_workflow"),
    ),
  ).toBe(false);
});
test("Keep on binds confirmation to the exact third assignment and same activation", async ({
  page,
}, info) => {
  await open(page, fixture(true, true));
  const card = page.getByTestId("dispatch-autopilot");
  await expect(
    card.getByText(
      "Autopilot made 3 assignments in the first hour. Keep it on?",
    ),
  ).toBeVisible();
  await waitForAnimations(page);
  await card.screenshot({
    path: info.outputPath("autopilot-confirmation.png"),
  });
  await card.getByRole("button", { name: "Keep on", exact: true }).click();
  await expect(
    card.getByRole("button", { name: "Keep on", exact: true }),
  ).toHaveCount(0, { timeout: 15000 });
  const confirmed = (await publishedRecords(page)).find(
    (r) => r.payload.type === "autopilot",
  );
  expect(confirmed.payload).toEqual({
    ...payload(),
    confirmation_required: false,
    confirmation_event: THIRD,
  });
  expect(confirmed.record_id).toBe(planningRecord(payload()).record.record_id);
  expect(confirmed.parent_event).toBe("a".repeat(64));
});
test("confirmation Pause leaves action history and links visible", async ({
  page,
}) => {
  const context = fixture(true, true);
  if (!context.automation) throw new Error("automation fixture missing");
  context.automation.actions = [
    {
      eventId: THIRD,
      signer: RELAY,
      createdAt: 3,
      issueId: context.issues[0].issueId,
      requestId: crypto.randomUUID(),
      action: "assign",
      result: "Assigned execution to Research helper",
    },
    {
      eventId: "2".repeat(64),
      signer: RELAY,
      createdAt: 2,
      issueId: context.issues[0].issueId,
      requestId: crypto.randomUUID(),
      action: "escalate",
      result: "Reminder posted",
    },
    {
      eventId: "1".repeat(64),
      signer: AGENT,
      createdAt: 5,
      issueId: context.issues[0].issueId,
      requestId: crypto.randomUUID(),
      action: "assign",
      result: "Untrusted assignment",
    },
  ];
  await open(page, context);
  await expect(
    page.getByText("Untrusted assignment", { exact: false }),
  ).toHaveCount(0);
  const rows = page.getByTestId("dispatch-autopilot").getByRole("listitem");
  await expect(rows.first()).toContainText("Assigned execution");
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  await expect(
    page.getByRole("switch", { name: "Autopilot", exact: true }),
  ).not.toBeChecked();
  await expect(rows).toHaveCount(2);
  await rows.first().getByRole("button").click();
  await expect(page).toHaveURL(/dispatch\/issue\//);
});
test("members and bots cannot authorize Autopilot or workspace alerts", async ({
  page,
}) => {
  const context = fixture();
  context.canAuthorizeAutomation = false;
  await open(page, context);
  await expect(
    page.getByRole("switch", { name: "Autopilot", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Settings", exact: true }),
  ).toHaveCount(0);
  expect(await publishedRecords(page)).toHaveLength(0);
});
test("workspace alert settings persist matching threshold and disable without deletion", async ({
  page,
}, info) => {
  await open(page, fixture(), false);
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const setting = page.getByTestId("dispatch-unowned-alert");
  await expect(setting.getByRole("switch")).toBeChecked({ timeout: 15000 });
  await expect(setting.getByLabel("after", { exact: true })).toHaveValue("24");
  await expect(setting.getByLabel("post in")).toHaveValue("");
  expect(await publishedRecords(page)).toHaveLength(0);
  await setting.getByLabel("after", { exact: true }).fill("12");
  await setting.getByRole("button", { name: "Save alert settings" }).click();
  await expect(
    setting.getByRole("button", { name: "Save alert settings" }),
  ).toBeDisabled();
  const first = (await publishedRecords(page))[0];
  expect(first.payload).toEqual({
    type: "unowned_alert",
    workspace: WORKSPACE,
    channel: null,
    after_hours: 12,
    enabled: true,
  });
  expect(
    (await workflows(page)).find(
      (w) => w.definition.name === "dispatch-unowned-alert",
    )?.definition,
  ).toMatchObject({
    enabled: true,
    steps: [{ id: "alert", action: "dispatch.escalate", after_hours: 12 }],
  });
  await waitForAnimations(page);
  await setting.screenshot({
    path: info.outputPath("unowned-alert-setting.png"),
  });
  await setting.getByRole("switch").click();
  await setting.getByRole("button", { name: "Save alert settings" }).click();
  await expect(
    setting.getByRole("button", { name: "Save alert settings" }),
  ).toBeDisabled();
  expect((await publishedRecords(page)).at(-1)).toMatchObject({
    record_id: first.record_id,
    revision: 1,
    payload: { enabled: false },
  });
  expect(
    (await workflows(page)).find(
      (w) => w.definition.name === "dispatch-unowned-alert",
    )?.definition.enabled,
  ).toBe(false);
});

test("a partially saved Pause survives card closure and retries without another record", async ({
  page,
}) => {
  await open(page);
  await chooseAgent(page);
  const toggle = page.getByRole("switch", { name: "Autopilot", exact: true });
  await toggle.click();
  await expect(toggle).toBeChecked({ timeout: 15000 });
  await page.evaluate(() => {
    if (window.__BUZZ_E2E__?.mock)
      window.__BUZZ_E2E__.mock.workflowUpdateError =
        "Workflow update unavailable";
  });
  await toggle.click();
  await expect(
    page.getByRole("alert").filter({ hasText: "Workflow update unavailable" }),
  ).toBeVisible();
  await page.getByRole("button", { name: /All project plans/ }).click();
  await page.getByRole("button", { name: /Website launch/ }).click();
  await expect(
    page.getByRole("button", { name: "Retry saved setting", exact: true }),
  ).toBeVisible();
  await page.evaluate(() => {
    if (window.__BUZZ_E2E__?.mock)
      window.__BUZZ_E2E__.mock.workflowUpdateError = undefined;
  });
  await page
    .getByRole("button", { name: "Retry saved setting", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Retry saved setting", exact: true }),
  ).toHaveCount(0);
  await expect(toggle).not.toBeChecked();
  expect(
    (await workflows(page)).find(
      (w) => w.definition.name === `dispatch-autopilot-${PROJECT}`,
    )?.definition.enabled,
  ).toBe(false);
  expect(
    (await publishedRecords(page)).filter(
      (r) => r.payload.type === "autopilot",
    ),
  ).toHaveLength(2);
});

test("lost authorization ACK recovers the accepted record and clears its signed retry entry", async ({
  page,
}) => {
  const context = fixture();
  context.records = [
    planningRecord(
      {
        type: "capacity",
        executor: AGENT,
        max_active: 2,
        capabilities: [],
        paused: false,
      },
      "b",
    ),
  ];
  await open(page, context);
  await chooseAgent(page);
  await page.evaluate(() => {
    window.__BUZZ_E2E_FAIL_DISPATCH_ACKS__ = 1;
  });
  await page.getByRole("switch", { name: "Autopilot", exact: true }).click();
  await expect(
    page.getByTestId("dispatch-autopilot").getByRole("alert"),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Retry saved setting", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Retry saved setting", exact: true }),
  ).toHaveCount(0);
  expect(
    (await publishedRecords(page)).filter(
      (r) => r.payload.type === "autopilot",
    ),
  ).toHaveLength(1);
  expect(
    await page.evaluate(() =>
      Object.keys(localStorage)
        .filter((key) => key.startsWith("buzz-dispatch-pending-v1:"))
        .every(
          (key) => JSON.parse(localStorage.getItem(key) ?? "[]").length === 0,
        ),
    ),
  ).toBe(true);
});
