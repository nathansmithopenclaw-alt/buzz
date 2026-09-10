import { expect, test } from "@playwright/test";

import type {
  DispatchContext,
  DispatchIssue,
} from "../../src/features/dispatch/types";
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

test("Dispatch recovers from a rate-limited history page through shared retry", async ({
  page,
}) => {
  const workspaceId = "11111111-1111-4111-8111-111111111111";
  await installMockBridge(
    page,
    {
      dispatchHistoryRateLimitCount: 1,
      dispatch: {
        workspace: { id: workspaceId, name: "Dispatch Test" },
        context: {
          issues: [],
          activity: [],
          comments: [],
          linkedTasks: [],
          projects: [],
          diagnostics: [],
          taskDiagnostics: [],
          viewerRole: "maintainer",
          sectionErrors: { comments: null, projects: null, tasks: null },
          snapshotAt: new Date().toISOString(),
        },
      },
    },
    { seedPreviewFeatures: true },
  );
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-dispatch-view").click();

  // Upstream #7188 retries a rate-limited history CLOSED inside the shared
  // relay client, before Dispatch's outer retry state observes a rejection.
  await expect(page.getByTestId("dispatch-new-issue")).toBeEnabled();
  await expect
    .poll(() =>
      page.evaluate(
        ({ workspaceId }) => {
          const trackerFilters =
            window.__BUZZ_E2E_DISPATCH_HISTORY_FILTERS__?.filter(
              (filter) =>
                filter["#h"]?.includes(workspaceId) &&
                filter.kinds?.includes(40020) &&
                filter.kinds?.includes(40021),
            ) ?? [];
          const counts = new Map<string, number>();
          for (const filter of trackerFilters) {
            const key = JSON.stringify(filter);
            counts.set(key, (counts.get(key) ?? 0) + 1);
          }
          return Math.max(0, ...counts.values());
        },
        { workspaceId },
      ),
    )
    .toBeGreaterThan(1);
});

test("Dispatch merges a live revision without refetching cold history", async ({
  page,
}) => {
  const workspaceId = "11111111-1111-4111-8111-111111111111";
  const issueId = "22222222-2222-4222-8222-222222222222";
  const context: DispatchContext = {
    issues: [],
    activity: [],
    comments: [],
    linkedTasks: [],
    projects: [],
    diagnostics: [],
    taskDiagnostics: [],
    viewerRole: "maintainer",
    sectionErrors: { comments: null, projects: null, tasks: null },
    snapshotAt: new Date().toISOString(),
  };
  await installMockBridge(
    page,
    {
      dispatch: {
        workspace: { id: workspaceId, name: "Dispatch Test" },
        context,
      },
    },
    { seedPreviewFeatures: true },
  );
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-dispatch-view").click();
  await expect(page.getByTestId("dispatch-new-issue")).toBeEnabled();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_COMMANDS__?.filter(
            (command) => command === "fold_tracker_context",
          ).length ?? 0,
      ),
    )
    .toBe(1);

  const liveContext: DispatchContext = {
    ...context,
    issues: [
      {
        streamId: workspaceId,
        issueId,
        rootEvent: "1".repeat(64),
        rootCreatedAt: 100,
        tipEvent: "2".repeat(64),
        tipCreatedAt: 101,
        revision: 1,
        enteredStatusAt: 101,
        identifier: "D-live",
        title: "Incremental revision reached the board",
        body: "Live projection fixture",
        status: "in_progress",
        priority: "P1",
        assignee: null,
        projectId: null,
        due: null,
        estimateMinutes: null,
        homeChannelId: null,
        workflow: "default",
        labels: ["live"],
        externalRefs: [],
        relations: [],
      },
    ],
    snapshotAt: new Date().toISOString(),
  };
  await page.evaluate(
    ({ workspaceId, liveContext }) => {
      window.__BUZZ_E2E_EMIT_MOCK_DISPATCH_EVENT__?.({
        workspaceId,
        context: liveContext,
        event: {
          id: "2".repeat(64),
          pubkey: "a".repeat(64),
          created_at: 101,
          kind: 40021,
          tags: [["h", workspaceId]],
          content: JSON.stringify({ issue_id: liveContext.issues[0].issueId }),
          sig: "b".repeat(128),
        },
      });
    },
    { workspaceId, liveContext },
  );

  await expect(
    page.getByText("Incremental revision reached the board"),
  ).toBeVisible();
  const foldCounts = await page.evaluate(() => ({
    cold:
      window.__BUZZ_E2E_COMMANDS__?.filter(
        (command) => command === "fold_tracker_context",
      ).length ?? 0,
    incremental:
      window.__BUZZ_E2E_COMMANDS__?.filter(
        (command) => command === "fold_tracker_context_incremental",
      ).length ?? 0,
  }));
  expect(foldCounts).toEqual({ cold: 1, incremental: 1 });
});

test("Dispatch warm start fetches only the persisted-cursor delta", async ({
  page,
}) => {
  const workspaceId = "11111111-1111-4111-8111-111111111111";
  const cursor = Math.floor(Date.now() / 1000) - 60;
  await installMockBridge(
    page,
    {
      dispatchWarmCursor: cursor,
      dispatch: {
        workspace: { id: workspaceId, name: "Dispatch Test" },
        context: {
          issues: [],
          activity: [],
          comments: [],
          linkedTasks: [],
          projects: [],
          diagnostics: [],
          taskDiagnostics: [],
          viewerRole: "maintainer",
          sectionErrors: { comments: null, projects: null, tasks: null },
          snapshotAt: new Date().toISOString(),
        },
      },
    },
    { seedPreviewFeatures: true },
  );
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-dispatch-view").click();
  await expect(page.getByTestId("dispatch-new-issue")).toBeEnabled();

  const proof = await page.evaluate(
    ({ workspaceId, cursor }) => ({
      commands: window.__BUZZ_E2E_COMMANDS__,
      deltaFilters: window.__BUZZ_E2E_DISPATCH_HISTORY_FILTERS__?.filter(
        (filter) =>
          (filter["#h"]?.includes(workspaceId) ||
            filter["#buzz-workspace"]?.includes(workspaceId)) &&
          filter.since === cursor,
      ),
      resumePayload: window.__BUZZ_E2E_COMMAND_PAYLOADS__?.find(
        ({ command }) => command === "resume_tracker_context",
      )?.payload,
    }),
    { workspaceId, cursor },
  );
  expect(
    proof.commands?.filter((command) => command === "fold_tracker_context"),
  ).toHaveLength(0);
  expect(
    proof.commands?.filter((command) => command === "resume_tracker_context"),
  ).toHaveLength(1);
  expect(proof.deltaFilters).toHaveLength(4);
  expect(proof.resumePayload).toMatchObject({ advanceCursor: true });
});

test("Dispatch warm-start timestamp gap falls back to a cold fold", async ({
  page,
}) => {
  const workspaceId = "11111111-1111-4111-8111-111111111111";
  const cursor = Math.floor(Date.now() / 1000) - 60;
  await installMockBridge(
    page,
    {
      dispatchWarmCursor: cursor,
      dispatchHistoryGapOnce: true,
      dispatch: {
        workspace: { id: workspaceId, name: "Dispatch Test" },
        context: {
          issues: [],
          activity: [],
          comments: [],
          linkedTasks: [],
          projects: [],
          diagnostics: [],
          taskDiagnostics: [],
          viewerRole: "maintainer",
          sectionErrors: { comments: null, projects: null, tasks: null },
          snapshotAt: new Date().toISOString(),
        },
      },
    },
    { seedPreviewFeatures: true },
  );
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-dispatch-view").click();
  await expect(page.getByTestId("dispatch-new-issue")).toBeEnabled();

  const proof = await page.evaluate(
    ({ workspaceId, cursor }) => {
      const trackerFilters =
        window.__BUZZ_E2E_DISPATCH_HISTORY_FILTERS__?.filter(
          (filter) =>
            filter["#h"]?.includes(workspaceId) &&
            filter.kinds?.includes(40020) &&
            filter.kinds?.includes(40021),
        ) ?? [];
      return {
        commands: window.__BUZZ_E2E_COMMANDS__,
        sawWarm: trackerFilters.some((filter) => filter.since === cursor),
        sawCold: trackerFilters.some((filter) => filter.since === undefined),
      };
    },
    { workspaceId, cursor },
  );
  expect(proof.sawWarm).toBe(true);
  expect(proof.sawCold).toBe(true);
  expect(
    proof.commands?.filter((command) => command === "resume_tracker_context"),
  ).toHaveLength(0);
  expect(
    proof.commands?.filter((command) => command === "fold_tracker_context"),
  ).toHaveLength(1);
});

test("New Issue creates a revision-zero issue visible in Board, List, and Triage", async ({
  page,
}) => {
  const workspaceId = "11111111-1111-4111-8111-111111111111";
  const projectId = "22222222-2222-4222-8222-222222222222";
  await installMockBridge(
    page,
    {
      dispatch: {
        workspace: { id: workspaceId, name: "Dispatch Test" },
        context: {
          issues: [],
          activity: [],
          comments: [],
          linkedTasks: [],
          projects: [{ projectId, slug: "desktop", name: "Desktop" }],
          diagnostics: [],
          taskDiagnostics: [],
          viewerRole: "maintainer",
          sectionErrors: { comments: null, projects: null, tasks: null },
          snapshotAt: new Date().toISOString(),
        },
      },
    },
    { seedPreviewFeatures: true },
  );
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-dispatch-view").click();
  await page.getByTestId("dispatch-new-issue").click();
  await page
    .getByTestId("dispatch-new-issue-title")
    .fill("Create from Dispatch UI");
  await page.getByTestId("dispatch-new-issue-body").fill("Canonical root body");
  await page.getByTestId("dispatch-new-issue-priority").selectOption("P1");
  await page.getByTestId("dispatch-new-issue-project").selectOption(projectId);
  await page.getByTestId("dispatch-new-issue-labels").fill("desktop, ui");
  await page.getByTestId("dispatch-new-issue-submit").click();

  await expect(page.getByTestId("dispatch-new-issue-dialog")).toHaveCount(0);
  await expect(page.getByText("Create from Dispatch UI")).toBeVisible();
  await page.getByTestId("dispatch-tab-list").click();
  await expect(
    page
      .getByTestId("dispatch-list-row")
      .filter({ hasText: "Create from Dispatch UI" }),
  ).toBeVisible();
  await page.getByRole("button", { name: /Triage/ }).click();
  await expect(page.getByText("Create from Dispatch UI")).toBeVisible();
  const signed = await page.evaluate(
    () => window.__BUZZ_E2E_SIGNED_EVENTS__ ?? [],
  );
  const root = signed.find((event) => event.kind === 40020);
  expect(root).toBeTruthy();
  const content = JSON.parse(root?.content ?? "{}") as {
    op?: string;
    status?: string;
    request_id?: string;
  };
  expect(content.op).toBe("create");
  expect(content.status).toBe("triage");
  expect(content.request_id).toBeTruthy();
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
      tipCreatedAt: 1,
      revision: 0,
      enteredStatusAt: 1,
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

    await expect(page.getByTestId("dispatch-workspace-label")).toContainText(
      "dispatch",
    );
    await expect(page.getByTestId("dispatch-board")).toBeVisible();
    await expect(page.getByTestId("dispatch-issue-card")).toHaveCount(5);
    await expect(page.getByTestId("dispatch-group-done")).toHaveCount(0);
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
      .filter({ hasText: "Long native identifier remains render safe" });
    const listIdentifier = longIdentifierRow.getByTestId(
      "dispatch-issue-identifier",
    );
    await expect(listIdentifier).toHaveAttribute("title", longIdentifier);
    const identifierGeometry = await longIdentifierRow.evaluate((row) => {
      const cells = row.querySelectorAll("td");
      const identifier = row.querySelector<HTMLElement>(
        '[data-testid="dispatch-issue-identifier"]',
      );
      if (!identifier || !cells[2])
        throw new Error("List cells are unavailable");
      return {
        isTruncated: identifier.scrollWidth > identifier.clientWidth,
        identifierRight: identifier.getBoundingClientRect().right,
        titleLeft: cells[2].getBoundingClientRect().left,
      };
    });
    expect(identifierGeometry.isTruncated).toBe(false);
    expect(identifierGeometry.identifierRight).toBeLessThanOrEqual(
      identifierGeometry.titleLeft + 1,
    );
    await page.getByTestId("dispatch-filter-query").fill("relay");
    await expect(page.getByTestId("dispatch-list-row")).toHaveCount(1);
    await expect(page.getByText("Relay coverage review")).toBeVisible();
  });
}

const ACCEPTANCE_WORKSPACE = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

function acceptanceIssue(
  overrides: Partial<DispatchIssue> &
    Pick<DispatchIssue, "identifier" | "issueId" | "title">,
): DispatchIssue {
  return {
    streamId: ACCEPTANCE_WORKSPACE,
    rootEvent: overrides.issueId.replaceAll("-", "").padEnd(64, "0"),
    rootCreatedAt: 100,
    tipEvent: overrides.issueId.replaceAll("-", "").padEnd(64, "1"),
    tipCreatedAt: 100,
    revision: 0,
    enteredStatusAt: 100,
    body: "Acceptance fixture",
    status: "backlog",
    priority: null,
    assignee: null,
    projectId: null,
    due: null,
    estimateMinutes: null,
    homeChannelId: ACCEPTANCE_WORKSPACE,
    workflow: "default",
    labels: ["acceptance"],
    externalRefs: [],
    relations: [],
    ...overrides,
  };
}

function acceptanceContext(issues: DispatchIssue[]): DispatchContext {
  return {
    issues,
    activity: [],
    comments: [],
    linkedTasks: [],
    projects: [
      {
        projectId: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
        slug: "desktop",
        name: "Desktop",
      },
    ],
    diagnostics: [],
    taskDiagnostics: [],
    viewerRole: "maintainer",
    sectionErrors: { comments: null, projects: null, tasks: null },
    snapshotAt: "2026-08-13T00:00:00Z",
  };
}

function orderedAcceptanceIssues(): DispatchIssue[] {
  return [
    acceptanceIssue({
      issueId: "00000000-0000-4000-8000-000000000004",
      identifier: "D-4",
      title: "Lane P1 newest",
      priority: "P1",
      tipCreatedAt: 500,
    }),
    acceptanceIssue({
      issueId: "00000000-0000-4000-8000-000000000002",
      identifier: "D-2",
      title: "Lane P0 older",
      priority: "P0",
      tipCreatedAt: 200,
    }),
    acceptanceIssue({
      issueId: "00000000-0000-4000-8000-000000000006",
      identifier: "D-6",
      title: "Lane no priority",
      priority: null,
      tipCreatedAt: 600,
    }),
    acceptanceIssue({
      issueId: "00000000-0000-4000-8000-000000000001",
      identifier: "D-1",
      title: "Lane P0 newest",
      priority: "P0",
      tipCreatedAt: 300,
    }),
    acceptanceIssue({
      issueId: "00000000-0000-4000-8000-000000000005",
      identifier: "D-5",
      title: "Lane P2 newest",
      priority: "P2",
      tipCreatedAt: 700,
    }),
    acceptanceIssue({
      issueId: "00000000-0000-4000-8000-000000000003",
      identifier: "D-3",
      title: "Lane P1 older",
      priority: "P1",
      tipCreatedAt: 400,
    }),
  ];
}

async function openAcceptanceDispatch(
  page: Parameters<typeof installMockBridge>[0],
  issues = orderedAcceptanceIssues(),
  workspaces?: Array<{ id: string; name: string; populated: boolean }>,
) {
  await installMockBridge(
    page,
    {
      dispatch: {
        workspace: { id: ACCEPTANCE_WORKSPACE, name: "dispatch" },
        workspaces,
        context: acceptanceContext(issues),
      },
    },
    { seedPreviewFeatures: true },
  );
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-dispatch-view").click();
  await expect(page.getByTestId("dispatch-board")).toBeVisible();
}

function routeSearch(url: string): URLSearchParams {
  const parsed = new URL(url);
  return new URLSearchParams(parsed.hash.split("?")[1] ?? parsed.search);
}

test("Board cards are ordered by priority then most-recently-updated", async ({
  page,
}) => {
  await openAcceptanceDispatch(page);

  const cards = page
    .getByTestId("dispatch-group-backlog")
    .getByTestId("dispatch-issue-card");
  await expect(cards).toHaveCount(6);
  await expect(cards.nth(0)).toContainText("Lane P0 newest");
  await expect(cards.nth(1)).toContainText("Lane P0 older");
  await expect(cards.nth(2)).toContainText("Lane P1 newest");
});

test("Board expands within the column and preserves filters", async ({
  page,
}) => {
  const issues = Array.from({ length: 24 }, (_, index) =>
    acceptanceIssue({
      issueId: `00000000-0000-4000-8000-${String(index).padStart(12, "0")}`,
      identifier: `D-${index}`,
      title: `Lane ${index}`,
    }),
  );
  await openAcceptanceDispatch(page, issues);
  await page.getByTestId("dispatch-filter-query").fill("lane");
  await page.getByTestId("dispatch-filter-label").selectOption("acceptance");
  const column = page.getByTestId("dispatch-group-backlog");
  await expect(column.getByTestId("dispatch-issue-card")).toHaveCount(20);
  await column.getByRole("button", { name: "Show 4 more" }).click();
  await expect(column.getByTestId("dispatch-issue-card")).toHaveCount(24);
  await expect(page.getByTestId("dispatch-board")).toBeVisible();
  expect(routeSearch(page.url()).get("label")).toBe("acceptance");
  expect(routeSearch(page.url()).get("query")).toBe("lane");
});

test("workspace control collapses one populated workspace and separates empty initialization", async ({
  page,
}) => {
  const emptyWorkspace = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
  await openAcceptanceDispatch(page, orderedAcceptanceIssues(), [
    { id: ACCEPTANCE_WORKSPACE, name: "dispatch", populated: true },
    { id: emptyWorkspace, name: "fresh-lab", populated: false },
  ]);

  await expect(page.getByTestId("dispatch-workspace-picker")).toHaveCount(0);
  await expect(page.getByTestId("dispatch-workspace-label")).toContainText(
    "dispatch",
  );
  await page.getByTestId("dispatch-empty-workspace-init").click();
  await expect(
    page.getByText("Existing issues stay in their current workspace."),
  ).toBeVisible();
  await page.getByTestId(`dispatch-init-workspace-${emptyWorkspace}`).click();
  await expect(page).toHaveURL(new RegExp(`workspace=${emptyWorkspace}`));
  await expect(page.getByTestId("dispatch-workspace-label")).toContainText(
    "Starting · fresh-lab",
  );
  await page.getByTestId("dispatch-view-populated-workspace").click();
  await expect(page.getByTestId("dispatch-workspace-label")).toContainText(
    "dispatch",
  );
});

test("Board and List URL state survives issue-detail back navigation", async ({
  page,
}) => {
  const projectId = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
  const issues = orderedAcceptanceIssues().map((issue, index) => ({
    ...issue,
    projectId,
    priority: index < 4 ? ("P1" as const) : issue.priority,
  }));
  await openAcceptanceDispatch(page, issues);
  await page.getByTestId("dispatch-filter-status").selectOption("backlog");
  await page.getByTestId("dispatch-filter-priority").selectOption("P1");
  await page.getByTestId("dispatch-filter-query").fill("lane");
  await page.getByTestId("dispatch-group-by").selectOption("project");
  await page.getByTestId("dispatch-tab-list").click();
  await page.getByTestId("dispatch-sort-title").click();
  await page.getByTestId("dispatch-sort-title").click();
  await page.getByTestId("dispatch-list-row").first().click();
  await expect(page.getByTestId("dispatch-issue-detail")).toBeVisible();

  await page.getByRole("button", { name: /^Dispatch ›/ }).click();

  await expect(page.getByTestId("dispatch-list")).toBeVisible();
  await expect(page.getByTestId("dispatch-filter-status")).toHaveValue(
    "backlog",
  );
  await expect(page.getByTestId("dispatch-filter-priority")).toHaveValue("P1");
  await expect(page.getByTestId("dispatch-filter-query")).toHaveValue("lane");
  const search = routeSearch(page.url());
  expect(search.get("view")).toBe("list");
  expect(search.get("group")).toBe("project");
  expect(search.get("sort")).toBe("title");
  expect(search.get("direction")).toBe("desc");
  await page.getByTestId("dispatch-tab-board").click();
  await expect(page.getByTestId("dispatch-group-by")).toHaveValue("project");
});
