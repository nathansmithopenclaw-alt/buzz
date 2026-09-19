import { expect, test } from "@playwright/test";
import {
  automationFixture,
  OWNER,
  WORKSPACE,
} from "../helpers/automationFixture";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

test("Triage badge counts unowned triage, shows oldest status age, and follows live assignment", async ({
  page,
}, testInfo) => {
  const now = Math.floor(Date.now() / 1000);
  await page.clock.setFixedTime(new Date(now * 1000));
  const context = automationFixture();
  const base = context.issues[0];
  context.issues = [
    {
      ...base,
      status: "triage",
      assignee: null,
      enteredStatusAt: now - 2 * 86400 - 60,
    },
    {
      ...base,
      issueId: "22222222-2222-4222-8222-222222222222",
      rootEvent: "2".repeat(64),
      tipEvent: "2".repeat(64),
      identifier: "D-young",
      title: "Younger unowned issue",
      status: "triage",
      assignee: null,
      enteredStatusAt: now - 3600,
    },
    {
      ...base,
      issueId: "33333333-3333-4333-8333-333333333333",
      rootEvent: "3".repeat(64),
      tipEvent: "3".repeat(64),
      identifier: "D-assigned",
      title: "Assigned triage issue",
      status: "triage",
      assignee: OWNER,
      enteredStatusAt: now - 10 * 86400,
    },
    {
      ...base,
      issueId: "44444444-4444-4444-8444-444444444444",
      rootEvent: "4".repeat(64),
      tipEvent: "4".repeat(64),
      identifier: "D-todo",
      title: "Unowned todo issue",
      assignee: null,
      enteredStatusAt: now - 20 * 86400,
    },
  ];
  await installMockBridge(page, {
    dispatch: { workspace: { id: WORKSPACE, name: "Product work" }, context },
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-dispatch-view").click();
  const nav = page.getByRole("navigation", { name: "Dispatch views" });
  await expect(nav.getByRole("button")).toHaveCount(5);
  await expect(page.getByTestId("dispatch-triage-badge")).toHaveText(
    "2 · oldest 2d",
  );
  await waitForAnimations(page);
  await nav.screenshot({ path: testInfo.outputPath("five-tab-bar.png") });
  await page.getByTestId("dispatch-tab-triage").click();
  await expect(
    page.getByRole("heading", { name: "Unassigned · 2" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Assigned · 1" }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page
    .getByTestId("dispatch-screen")
    .screenshot({ path: testInfo.outputPath("triage-badge.png") });
  for (const [index, expected] of [
    [0, "1 · oldest 0d"],
    [1, null],
  ] as const) {
    context.issues[index].assignee = OWNER;
    context.issues[index].revision += 1;
    await page.evaluate(
      ({ context, workspaceId, index, now }) => {
        window.__BUZZ_E2E_EMIT_MOCK_DISPATCH_EVENT__?.({
          workspaceId,
          context,
          event: {
            id: (index === 0 ? "e" : "f").repeat(64),
            pubkey: "a".repeat(64),
            created_at: now + index,
            kind: 40021,
            tags: [["h", workspaceId]],
            content: JSON.stringify({
              issue_id: context.issues[index].issueId,
            }),
            sig: "b".repeat(128),
          },
        });
      },
      { context, workspaceId: WORKSPACE, index, now },
    );
    if (expected)
      await expect(page.getByTestId("dispatch-triage-badge")).toHaveText(
        expected,
      );
    else await expect(page.getByTestId("dispatch-triage-badge")).toHaveCount(0);
  }
  await expect(
    page.getByRole("heading", { name: "Unassigned · 0" }),
  ).toBeVisible();
});
