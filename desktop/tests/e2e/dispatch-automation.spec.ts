import { expect, test } from "@playwright/test";
import {
  AGENT,
  ISSUE,
  OWNER,
  SERVICE,
  WORKSPACE,
  automationFixture,
  openAutomation,
  planningRecord,
  publishedRecords,
} from "../helpers/automationFixture";

test("first visit explains the jobs and offers an honest connection next step", async ({
  page,
}) => {
  const context = automationFixture();
  context.records = [];
  context.automation = { actions: [], heartbeats: [] };
  await openAutomation(page, context);
  await expect(
    page.getByRole("heading", { name: "Let Buzz handle the routine" }),
  ).toBeVisible();
  await expect(
    page.getByText("Nothing is running yet", { exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel("How automation works")).toContainText(
    "Review, then turn it on",
  );
  await page.getByRole("button", { name: /^Save messages as work/ }).click();
  const dialog = page.getByTestId("automation-setup");
  await expect(
    dialog.getByRole("checkbox", {
      name: "Save messages as work",
      exact: true,
    }),
  ).toBeChecked();
  await dialog
    .getByRole("button", { name: "Choose the details", exact: true })
    .click();
  await expect(
    dialog.getByText(
      /Ask the person who manages your Buzz workspace to connect automation/,
    ),
  ).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "Review setup", exact: true }),
  ).toBeDisabled();
  await expect(
    dialog.getByLabel("Service public key", { exact: true }),
  ).not.toBeVisible();
  await dialog.getByRole("button", { name: "Back", exact: true }).click();
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(await publishedRecords(page)).toHaveLength(0);
});

test("capture setup reuses its connection, shows named scope, and saves only a preview", async ({
  page,
}, testInfo) => {
  await openAutomation(page);
  await page.getByRole("button", { name: /^Save messages as work/ }).click();
  const dialog = page.getByTestId("automation-setup");
  await dialog
    .getByRole("button", { name: "Choose the details", exact: true })
    .click();
  await expect(
    dialog.getByLabel("Service public key", { exact: true }),
  ).not.toBeVisible();
  await dialog.getByRole("checkbox", { name: "#general", exact: true }).check();
  await dialog.getByLabel("Stop after this many actions").fill("2");
  await dialog
    .getByRole("button", { name: "Review setup", exact: true })
    .click();
  await expect(dialog.getByTestId("automation-rule-summary")).toContainText(
    "In #general, Buzz creates an issue",
  );
  await expect(dialog).toContainText("This replaces the previous setup");
  await page.screenshot({ path: testInfo.outputPath("capture-review.png") });
  await dialog.getByRole("button", { name: "Back", exact: true }).click();
  await expect(
    dialog.getByRole("checkbox", { name: "#general", exact: true }),
  ).toBeChecked();
  await expect(dialog.getByLabel("Stop after this many actions")).toHaveValue(
    "2",
  );
  await dialog
    .getByRole("button", { name: "Review setup", exact: true })
    .click();
  await dialog
    .getByRole("button", { name: "Save and check setup", exact: true })
    .click();
  await expect(dialog).not.toBeVisible();
  await expect(
    page.getByText(/Setup saved. Buzz is checking it/),
  ).toBeVisible();
  const records = await publishedRecords(page);
  expect(records).toHaveLength(1);
  expect(records[0].payload.controls).toMatchObject({
    service_pubkey: SERVICE,
    mode: "preview",
    max_actions_total: 2,
    routing: [],
    issue_ids: [],
    unassigned_after_secs: null,
  });
  expect(records[0].payload.controls.capture_channels).toHaveLength(1);
  await expect(
    page.getByRole("button", { name: "Turn on…", exact: true }),
  ).toBeDisabled();
  expect(
    await page.evaluate(() =>
      (window.__BUZZ_E2E_PUBLISHED_DISPATCH_EVENTS__ ?? []).filter(
        (event) => event.kind === 9,
      ),
    ),
  ).toHaveLength(0);
});

test("turning on requires a separate confirmation and publishes one bounded revision", async ({
  page,
}) => {
  await openAutomation(
    page,
    automationFixture({ notify_triage: true, issue_ids: [ISSUE] }),
  );
  const card = page.getByTestId("automation-run-card");
  await expect(card.getByRole("status")).toHaveText("Ready to turn on");
  await card.getByRole("button", { name: "Turn on…", exact: true }).click();
  const confirm = page.getByRole("dialog", {
    name: "Turn on this automation?",
    exact: true,
  });
  await expect(confirm).toContainText("3 more actions");
  await confirm.getByRole("button", { name: "Not yet", exact: true }).click();
  expect(await publishedRecords(page)).toHaveLength(0);
  await card.getByRole("button", { name: "Turn on…", exact: true }).click();
  const before = Math.floor(Date.now() / 1000);
  await confirm
    .getByRole("button", { name: "Turn on for up to 1 hour", exact: true })
    .click();
  await expect(confirm).not.toBeVisible();
  const records = await publishedRecords(page);
  expect(records).toHaveLength(1);
  expect(records[0].revision).toBe(1);
  expect(records[0].parent_event).toBe("a".repeat(64));
  expect(records[0].payload.controls.mode).toBe("active");
  expect(records[0].payload.controls.expires_at).toBeGreaterThanOrEqual(
    before + 3600,
  );
  expect(records[0].payload.controls.expires_at).toBeLessThanOrEqual(
    Math.floor(Date.now() / 1000) + 3600,
  );
  await expect(card.getByRole("status")).toHaveText("Checking setup…");
});

test("a lost connection explains why a preview cannot start", async ({
  page,
}) => {
  const context = automationFixture({
    issue_ids: [ISSUE],
    notify_triage: true,
  });
  if (!context.automation) throw new Error("Missing automation fixture");
  context.automation.heartbeats[0].observedAt -= 600;
  await openAutomation(page, context);
  await expect(
    page.getByTestId("automation-run-card").getByRole("status"),
  ).toHaveText("Connection lost");
  await expect(
    page.getByRole("button", { name: "Turn on…", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByText("Buzz cannot confirm what is happening right now.", {
      exact: false,
    }),
  ).toBeVisible();
});

test("captured work is visible in activity and a spent run cannot be restarted", async ({
  page,
}, testInfo) => {
  const context = automationFixture({
    mode: "paused",
    capture_channels: [WORKSPACE],
    max_actions_total: 1,
  });
  if (!context.automation) throw new Error("Missing automation fixture");
  context.automation.heartbeats[0].health.detail = JSON.stringify({
    approval: "a".repeat(64),
    controls: "paused",
    spent: 1,
    pending: 0,
  });
  context.issues[0].sourceEventId = "f".repeat(64);
  context.activity = [
    {
      issueId: ISSUE,
      eventId: context.issues[0].rootEvent,
      authorPubkey: SERVICE,
      createdAt: context.issues[0].rootCreatedAt,
      revision: 0,
      reason: null,
      requestId: null,
      import: null,
      changes: [],
    },
  ];
  await openAutomation(page, context);
  await expect(
    page.getByText("1 of 1 actions used", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Created from a message", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Check before turning on", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Turn on…", exact: true }),
  ).toHaveCount(0);
  await page.screenshot({
    path: testInfo.outputPath("automation-overview.png"),
    fullPage: true,
  });
  await page
    .getByRole("button", { name: "Review another run", exact: true })
    .click();
  await expect(page.getByTestId("automation-setup")).toBeVisible();
  expect(await publishedRecords(page)).toHaveLength(0);
});

test("agent setup uses names and excludes people from the agent picker", async ({
  page,
}) => {
  const context = automationFixture();
  context.records?.push(
    planningRecord(
      {
        type: "capacity",
        executor: AGENT,
        max_active: 1,
        capabilities: ["research"],
        paused: false,
      },
      "d",
    ),
  );
  await openAutomation(page, context);
  await page.getByRole("button", { name: /^Send work to an agent/ }).click();
  const dialog = page.getByTestId("automation-setup");
  await dialog
    .getByRole("button", { name: "Choose the details", exact: true })
    .click();
  await dialog
    .getByRole("checkbox", {
      name: "Research a simpler onboarding flow",
      exact: true,
    })
    .check();
  await dialog.getByLabel("Assignment 1 label").selectOption("research");
  await dialog
    .getByLabel("Assignment 1 agent")
    .selectOption({ label: "Research helper" });
  await dialog
    .getByLabel("Assignment 1 channel")
    .selectOption({ label: "#general" });
  await dialog
    .getByRole("button", { name: "Add an agent", exact: true })
    .click();
  const capacity = page.getByRole("region", {
    name: "Agent work settings",
    exact: true,
  });
  await expect(
    dialog.getByRole("button", { name: "Review setup", exact: true }),
  ).toBeDisabled();
  await capacity
    .getByRole("button", { name: "Choose an agent", exact: true })
    .click();
  const picker = page.getByTestId("dispatch-assignee-picker");
  await expect(
    picker.getByTestId(`dispatch-assignee-option-${AGENT}`),
  ).toBeVisible();
  await expect(
    picker.getByTestId(`dispatch-assignee-option-${OWNER}`),
  ).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(picker).not.toBeVisible();
  await expect(capacity).toBeVisible();
  await capacity
    .getByRole("button", { name: "Cancel agent settings", exact: true })
    .click();
  await expect(capacity).not.toBeVisible();
  await expect(dialog).toBeVisible();
  await dialog
    .getByRole("button", { name: "Review setup", exact: true })
    .click();
  await expect(dialog.getByTestId("automation-rule-summary")).toContainText(
    "Send it to Research helper in #general",
  );
  await expect(dialog).toContainText("The issue owner reviews the result");
  expect(await publishedRecords(page)).toHaveLength(0);
});

test("reminder setup stays usable in a narrow window with larger text", async ({
  page,
}, testInfo) => {
  await openAutomation(page);
  await page
    .getByRole("button", { name: /^Remind us about unassigned work/ })
    .click();
  await page.setViewportSize({ width: 375, height: 812 });
  await page.evaluate(() => {
    document.documentElement.style.fontSize = "20px";
  });
  const dialog = page.getByTestId("automation-setup");
  await dialog
    .getByRole("button", { name: "Choose the details", exact: true })
    .click();
  await dialog
    .getByRole("checkbox", {
      name: "Research a simpler onboarding flow",
      exact: true,
    })
    .check();
  await dialog.getByLabel("Waiting time in hours").fill("48");
  await dialog
    .getByLabel("Post reminders in")
    .selectOption({ label: "#general" });
  await dialog
    .getByRole("button", { name: "Review setup", exact: true })
    .click();
  await expect(dialog.getByTestId("automation-rule-summary")).toContainText(
    "after 48 hours",
  );
  const bounds = await dialog.boundingBox();
  expect(bounds?.x).toBeGreaterThanOrEqual(0);
  expect((bounds?.x ?? 0) + (bounds?.width ?? 0)).toBeLessThanOrEqual(375);
  const save = dialog.getByRole("button", {
    name: "Save and check setup",
    exact: true,
  });
  await expect(save).toBeInViewport();
  expect(
    await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth + 1),
  ).toBe(true);
  await page.screenshot({ path: testInfo.outputPath("narrow-review.png") });
  await save.click();
  await expect(dialog).not.toBeVisible();
  const records = await publishedRecords(page);
  expect(records[0].payload.controls).toMatchObject({
    mode: "preview",
    issue_ids: [ISSUE],
    unassigned_after_secs: 172800,
    capture_channels: [],
    routing: [],
  });
});

test("dark setup remains readable and its actions reachable with 200 percent text", async ({
  page,
}, testInfo) => {
  await page.addInitScript(() =>
    window.localStorage.setItem("buzz-theme", "catppuccin-mocha"),
  );
  await page.setViewportSize({ width: 1280, height: 900 });
  await openAutomation(page);
  await page.getByRole("button", { name: /^Save messages as work/ }).click();
  await page.evaluate(() => {
    document.documentElement.style.fontSize = "32px";
  });
  const dialog = page.getByTestId("automation-setup");
  await dialog
    .getByRole("button", { name: "Choose the details", exact: true })
    .click();
  await dialog.getByRole("checkbox", { name: "#general", exact: true }).check();
  await dialog
    .getByRole("button", { name: "Review setup", exact: true })
    .click();
  await expect(
    dialog.getByRole("button", { name: "Save and check setup", exact: true }),
  ).toBeInViewport();
  expect(
    await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth + 1),
  ).toBe(true);
  await page.screenshot({
    path: testInfo.outputPath("dark-large-text-review.png"),
  });
  await dialog.getByRole("button", { name: "Back", exact: true }).click();
  await expect(
    dialog.getByRole("checkbox", { name: "#general", exact: true }),
  ).toBeChecked();
  expect(await publishedRecords(page)).toHaveLength(0);
});
