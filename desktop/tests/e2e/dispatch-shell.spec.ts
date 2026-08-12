import { expect, test } from "@playwright/test";

import type { DispatchIssue } from "../../src/features/dispatch/types";
import { installMockBridge } from "../helpers/bridge";

test("Dispatch experiment enables the sidebar route shell", async ({
  page,
}) => {
  await installMockBridge(page, undefined, { seedPreviewFeatures: false });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  await expect(page.getByTestId("open-dispatch-view")).toHaveCount(0);
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-experimental").click();
  await page.getByTestId("feature-toggle-dispatch").click();
  await page.getByTestId("settings-back-to-app").click();
  await expect(page.getByTestId("open-dispatch-view")).toBeVisible();

  await page.getByTestId("open-dispatch-view").click();
  await expect(page).toHaveURL(/\/dispatch$/);
  await expect(page.getByTestId("dispatch-screen")).toBeVisible();
  await expect(
    page.getByText("No Tracker-enabled member workspaces were found"),
  ).toBeVisible();
});

for (const theme of ["catppuccin-latte", "catppuccin-mocha"] as const) {
  test(`Dispatch board and list render canonical fixture projections in ${theme}`, async ({
    page,
  }) => {
    const workspaceId = "11111111-1111-4111-8111-111111111111";
    const projectId = "22222222-2222-4222-8222-222222222222";
    const assignee = "a".repeat(64);
    const longIdentifier = `D-${"b".repeat(32)}`;
    const makeIssue = (overrides: Partial<DispatchIssue>): DispatchIssue => ({
      streamId: workspaceId,
      issueId: crypto.randomUUID(),
      rootEvent: "1".repeat(64),
      rootCreatedAt: 1,
      tipEvent: "2".repeat(64),
      identifier: "SMI-1",
      title: "Dispatch fixture issue",
      body: "Fixture body",
      status: "backlog",
      priority: null,
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
    });
    const blockedId = crypto.randomUUID();
    await page.addInitScript((selectedTheme) => {
      window.localStorage.setItem("buzz-theme", selectedTheme);
    }, theme);
    await installMockBridge(
      page,
      {
        dispatch: {
          workspace: { id: workspaceId, name: "dispatch" },
          context: {
            issues: [
              makeIssue({
                issueId: crypto.randomUUID(),
                identifier: longIdentifier,
                title: "Long native identifier remains render safe",
                status: "triage",
                priority: "P0",
                labels: ["native"],
                relations: [
                  {
                    kind: "blocks",
                    targetIssueId: blockedId,
                    targetRootEvent: "3".repeat(64),
                  },
                ],
              }),
              makeIssue({
                issueId: blockedId,
                identifier: "SMI-48",
                title: "Add Platinum plan tier",
                projectId,
                labels: ["plans"],
              }),
              makeIssue({
                identifier: "SMI-231",
                title: "Publish state machine redesign",
                status: "in_progress",
                priority: "P1",
                assignee,
              }),
              makeIssue({
                identifier: "SMI-210",
                title: "Relay coverage review",
                status: "in_review",
                priority: "P2",
              }),
              makeIssue({
                identifier: "SMI-309",
                title: "Todo remains visible on the board",
                status: "todo",
                priority: "P3",
              }),
              makeIssue({
                identifier: "SMI-8",
                title: "Wake refactor complete",
                status: "done",
                assignee,
              }),
            ],
            projects: [{ projectId, slug: "plans", name: "Plans" }],
            diagnostics: [
              { kind: "LosingBranch", message: "fixture diagnostic" },
            ],
            snapshotAt: "2026-08-12T12:24:00Z",
          },
        },
        searchProfiles: [
          { pubkey: assignee, displayName: "Patch", avatarUrl: null },
        ],
      },
      { seedPreviewFeatures: true },
    );
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await page.getByTestId("open-dispatch-view").click();

    await expect(page.getByTestId("dispatch-workspace-picker")).toHaveValue(
      workspaceId,
    );
    await expect(page.getByTestId("dispatch-board")).toBeVisible();
    await expect(page.getByTestId("dispatch-issue-card")).toHaveCount(6);
    await expect(page.getByTestId("dispatch-group-todo")).toBeVisible();
    await page.getByTestId("dispatch-filter-status").selectOption("todo");
    await expect(page.getByTestId("dispatch-issue-card")).toHaveCount(1);
    await expect(
      page.getByText("Todo remains visible on the board"),
    ).toBeVisible();
    await page.getByTestId("dispatch-filter-query").fill("no such issue");
    await expect(
      page.getByText("No issues match these filters."),
    ).toBeVisible();
    await page.getByTestId("dispatch-filter-query").fill("");
    await page.getByTestId("dispatch-filter-status").selectOption("all");
    await expect(
      page.getByTestId("dispatch-diagnostics-warning"),
    ).toBeVisible();
    await expect(
      page.getByTestId("dispatch-issue-identifier").filter({ hasText: "D-" }),
    ).toHaveAttribute("title", longIdentifier);

    await page
      .getByTestId("dispatch-filter-status")
      .selectOption("in_progress");
    await expect(page.getByTestId("dispatch-issue-card")).toHaveCount(1);
    await expect(
      page.getByText("Publish state machine redesign"),
    ).toBeVisible();
    await page.getByTestId("dispatch-filter-status").selectOption("all");
    await page.getByTestId("dispatch-group-by").selectOption("priority");
    await expect(page.getByTestId("dispatch-group-P1")).toBeVisible();

    await page.getByTestId("dispatch-tab-list").click();
    await expect(page.getByTestId("dispatch-list-row")).toHaveCount(6);
    const longIdentifierRow = page
      .getByTestId("dispatch-list-row")
      .filter({ hasText: longIdentifier });
    const listIdentifier = longIdentifierRow.getByTestId(
      "dispatch-issue-identifier",
    );
    await expect(listIdentifier).toHaveAttribute("title", longIdentifier);
    const identifierGeometry = await longIdentifierRow.evaluate((row) => {
      const cells = row.querySelectorAll("td");
      const identifier = cells[0]?.querySelector<HTMLElement>(
        '[data-testid="dispatch-issue-identifier"]',
      );
      if (!identifier || !cells[1])
        throw new Error("List cells are unavailable");
      return {
        isTruncated: identifier.scrollWidth > identifier.clientWidth,
        identifierRight: identifier.getBoundingClientRect().right,
        titleLeft: cells[1].getBoundingClientRect().left,
      };
    });
    expect(identifierGeometry.isTruncated).toBe(true);
    expect(identifierGeometry.identifierRight).toBeLessThanOrEqual(
      identifierGeometry.titleLeft + 1,
    );
    await page.getByTestId("dispatch-filter-query").fill("relay");
    await expect(page.getByTestId("dispatch-list-row")).toHaveCount(1);
    await expect(page.getByText("Relay coverage review")).toBeVisible();
  });
}
