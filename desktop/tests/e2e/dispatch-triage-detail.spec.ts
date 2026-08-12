import { expect, test } from "@playwright/test";

import type {
  DispatchContext,
  DispatchIssue,
} from "../../src/features/dispatch/types";
import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const WORKSPACE = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const SELF = TEST_IDENTITIES.tyler.pubkey;
const OTHER = TEST_IDENTITIES.alice.pubkey;
const LONG_IDENTIFIER = `D-${"a".repeat(32)}`;
const RELATED_LONG_IDENTIFIER = `D-${"b".repeat(32)}`;

function issue(
  overrides: Partial<DispatchIssue> &
    Pick<DispatchIssue, "issueId" | "identifier" | "title">,
): DispatchIssue {
  return {
    streamId: WORKSPACE,
    rootEvent: "1".repeat(64),
    rootCreatedAt: 1_720_000_000,
    tipEvent: "1".repeat(64),
    tipCreatedAt: 1_720_000_000,
    revision: 0,
    enteredStatusAt: 1_720_000_000,
    body: "A canonical issue description.",
    status: "triage",
    priority: "P2",
    assignee: null,
    projectId: "project-1",
    due: null,
    estimateMinutes: null,
    homeChannelId: WORKSPACE,
    workflow: "default",
    labels: ["dispatch"],
    externalRefs: [],
    relations: [],
    ...overrides,
  };
}

function context(overrides: Partial<DispatchContext> = {}): DispatchContext {
  const primary = issue({
    issueId: "11111111-1111-4111-8111-111111111111",
    identifier: LONG_IDENTIFIER,
    title: "P0 oldest canonical triage issue",
    priority: "P0",
    enteredStatusAt: 1_710_000_000,
    body: "| Check | Result |\n| --- | --- |\n| Fold | Canonical |",
    relations: [
      {
        kind: "blocks",
        targetIssueId: "44444444-4444-4444-8444-444444444444",
        targetRootEvent: "4".repeat(64),
      },
    ],
    externalRefs: [{ type: "linear", ref: "SMI-281" }],
  });
  const issues = [
    issue({
      issueId: "22222222-2222-4222-8222-222222222222",
      identifier: "SMI-282",
      title: "P1 older than second P0",
      priority: "P1",
      enteredStatusAt: 1_700_000_000,
    }),
    issue({
      issueId: "33333333-3333-4333-8333-333333333333",
      identifier: "SMI-283",
      title: "P0 newer canonical issue",
      priority: "P0",
      enteredStatusAt: 1_715_000_000,
    }),
    primary,
    issue({
      issueId: "44444444-4444-4444-8444-444444444444",
      identifier: RELATED_LONG_IDENTIFIER,
      title: "Assigned active issue",
      status: "triage",
      priority: "P2",
      assignee: SELF,
      due: { value: 1_900_000_000, precision: "date", timezone: "UTC" },
    }),
  ];
  return {
    issues,
    activity: [
      {
        issueId: primary.issueId,
        eventId: primary.rootEvent,
        authorPubkey: OTHER,
        createdAt: primary.rootCreatedAt,
        revision: 0,
        reason: null,
        requestId: null,
        import: {
          system: "linear",
          external_id: "SMI-281",
          source_url: "https://linear.app/example",
          imported_at: 1_720_000_000,
          importer_version: "buzz-linear-tracker-v1",
        },
        changes: [],
      },
      {
        issueId: primary.issueId,
        eventId: "5".repeat(64),
        authorPubkey: OTHER,
        createdAt: 1_720_000_100,
        revision: 1,
        reason: "Needs owner review",
        requestId: null,
        import: null,
        changes: [{ field: "status", before: "backlog", after: "triage" }],
      },
    ],
    comments: [
      {
        issueId: primary.issueId,
        eventId: "6".repeat(64),
        authorPubkey: OTHER,
        createdAt: 1_720_000_200,
        rootEvent: primary.rootEvent,
        parentEvent: null,
        body: "Canonical comment from the fold.",
        requestId: null,
        import: null,
      },
    ],
    linkedTasks: [
      {
        issueId: primary.issueId,
        taskId: "77777777-7777-4777-8777-777777777777",
        title: "Investigate canonical issue",
        status: "picked_up",
        assignee: SELF,
        eventId: "7".repeat(64),
        channelId: WORKSPACE,
        createdAt: 1_720_000_300,
        dispatchRequestId: null,
      },
    ],
    projects: [
      { projectId: "project-1", slug: "dispatch", name: "Dispatch Desktop" },
    ],
    diagnostics: [],
    taskDiagnostics: [],
    viewerRole: "maintainer",
    sectionErrors: { comments: null, projects: null, tasks: null },
    snapshotAt: new Date(1_720_000_300 * 1_000).toISOString(),
    ...overrides,
  };
}

async function openDispatch(
  page: import("@playwright/test").Page,
  dispatchContext = context(),
) {
  await installMockBridge(page, {
    dispatch: {
      workspace: { id: WORKSPACE, name: "Dispatch Test Workspace" },
      context: dispatchContext,
    },
    searchProfiles: [{ pubkey: OTHER, displayName: "Helm" }],
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-dispatch-view").click();
  await expect(page.getByTestId("dispatch-screen")).toBeVisible();
}

async function openPrimaryIssue(page: import("@playwright/test").Page) {
  await page.getByRole("button", { name: /Triage/ }).click();
  await page
    .getByTestId("dispatch-issue-11111111-1111-4111-8111-111111111111")
    .click();
  await expect(page.getByTestId("dispatch-issue-detail")).toBeVisible();
}

test("triage renders unassigned and assigned current-fold sections in P0-first then-oldest order", async ({
  page,
}) => {
  await openDispatch(page);
  await page.getByRole("button", { name: /Triage/ }).click();
  await expect(page.getByTestId("dispatch-triage")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Unassigned · 3" }),
  ).toBeVisible();
  const rows = page.locator('[data-testid^="dispatch-issue-"]');
  await expect(rows.nth(0)).toContainText("P0 oldest canonical triage issue");
  await expect(rows.nth(1)).toContainText("P0 newer canonical issue");
  await expect(rows.nth(2)).toContainText("P1 older than second P0");
  await expect(
    page.getByText(
      /Escalation thresholds and auto-routing arrive with the P2 dispatcher/,
    ),
  ).toBeVisible();
});

test("detail renders markdown, chronological comments, relations, and linked task envelopes", async ({
  page,
}) => {
  await openDispatch(page);
  await openPrimaryIssue(page);
  await expect(page.getByRole("table")).toContainText("Canonical");
  await expect(page.getByTestId("dispatch-comment")).toContainText(
    "Canonical comment from the fold.",
  );
  await expect(page.getByText("Needs owner review")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Relations" })).toBeVisible();
  const breadcrumbIdentifier = page.getByTitle(LONG_IDENTIFIER);
  const relationIdentifier = page.getByTitle(RELATED_LONG_IDENTIFIER);
  await expect(breadcrumbIdentifier).toBeVisible();
  await expect(relationIdentifier).toBeVisible();
  for (const identifier of [breadcrumbIdentifier, relationIdentifier]) {
    await expect(identifier).toHaveCSS("text-overflow", "ellipsis");
    expect(
      await identifier.evaluate(
        (element) => element.scrollWidth > element.clientWidth,
      ),
    ).toBe(true);
  }
  await expect(page.getByText("Investigate canonical issue")).toBeVisible();
});

test("detail mutation round-trips through the Rust-builder and canonical refresh fixture", async ({
  page,
}) => {
  await openDispatch(page);
  await openPrimaryIssue(page);
  const status = page.getByLabel("Issue status");
  await status.selectOption("in_review");
  await expect(page.getByText("Issue updated")).toBeVisible();
  await expect(status).toHaveValue("in_review");
  const signed = await page.evaluate(
    () => window.__BUZZ_E2E_SIGNED_EVENTS__ ?? [],
  );
  expect(
    signed.some(
      (event) =>
        event.kind === 40021 &&
        JSON.parse(event.content).set.status === "in_review",
    ),
  ).toBe(true);
});

test("detail keeps canonical activity visible when comments and profiles degrade", async ({
  page,
}) => {
  const degraded = context({
    sectionErrors: {
      comments: "comment relay timed out",
      projects: null,
      tasks: null,
    },
  });
  await installMockBridge(page, {
    dispatch: {
      workspace: { id: WORKSPACE, name: "Dispatch Test Workspace" },
      context: degraded,
    },
    usersBatchError: "profile hydration failed",
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-dispatch-view").click();
  await openPrimaryIssue(page);
  await expect(page.getByText(/Comments are unavailable/)).toBeVisible();
  await expect(
    page.getByText(/Some profile names are unavailable/),
  ).toBeVisible();
  await expect(page.getByText("Needs owner review")).toBeVisible();
  await expect(page.getByTestId("dispatch-comment")).toHaveCount(0);
});
