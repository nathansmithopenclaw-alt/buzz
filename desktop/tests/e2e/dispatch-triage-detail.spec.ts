import {
  automationFixture,
  openAutomation,
  publishedRecords,
  ISSUE as AUTOMATION_ISSUE,
} from "../helpers/automationFixture";
import { expect, test } from "@playwright/test";

import type {
  DispatchContext,
  DispatchIssue,
} from "../../src/features/dispatch/types";
import { waitForAnimations } from "../helpers/animations";
import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const WORKSPACE = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const SELF = TEST_IDENTITIES.tyler.pubkey;
const OTHER = TEST_IDENTITIES.alice.pubkey;
const PRIMARY_ISSUE = "11111111-1111-4111-8111-111111111111";
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
  dispatchOptions: {
    rejectNextRevision?: boolean;
    members?: string[];
    assignmentAckDelayMs?: number;
  } = {},
) {
  await installMockBridge(page, {
    dispatch: {
      workspace: { id: WORKSPACE, name: "Dispatch Test Workspace" },
      context: dispatchContext,
      ...dispatchOptions,
    },
    searchProfiles: [
      { pubkey: OTHER, displayName: "Helm" },
      { pubkey: SELF, displayName: "Tyler" },
    ],
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-dispatch-view").click();
  await expect(page.getByTestId("dispatch-screen")).toBeVisible();
}

async function openPrimaryIssue(page: import("@playwright/test").Page) {
  await page
    .getByRole("button", { name: /^Triage(?: \d+ · oldest \d+d)?$/ })
    .click();
  await page
    .getByTestId("dispatch-issue-11111111-1111-4111-8111-111111111111")
    .click();
  await expect(page.getByTestId("dispatch-issue-detail")).toBeVisible();
}

test("triage renders unassigned and assigned current-fold sections in P0-first then-oldest order", async ({
  page,
}) => {
  await openDispatch(page);
  await page
    .getByRole("button", { name: /^Triage(?: \d+ · oldest \d+d)?$/ })
    .click();
  await expect(page.getByTestId("dispatch-triage")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Unassigned · 3" }),
  ).toBeVisible();
  const rows = page.locator('[data-testid^="dispatch-issue-"]');
  await expect(rows.nth(0)).toContainText("P0 oldest canonical triage issue");
  await expect(rows.nth(1)).toContainText("P0 newer canonical issue");
  await expect(rows.nth(2)).toContainText("P1 older than second P0");
  await expect(
    page.getByText(/Open an issue to review its details/),
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
  await expect(breadcrumbIdentifier).toHaveText("D-AAAAAA");
  await expect(relationIdentifier).toHaveCSS("text-overflow", "ellipsis");
  await expect(page.getByText("Investigate canonical issue")).toBeVisible();
});

test("detail mutation round-trips through the Rust-builder and canonical refresh fixture", async ({
  page,
}) => {
  await openDispatch(page);
  await openPrimaryIssue(page);
  await page.getByTestId("dispatch-status-reason").fill("Needs owner review");
  const status = page.getByLabel("Issue status");
  await status.selectOption("in_review");
  await expect(page.getByText("Issue updated")).toBeVisible();
  await expect(status).toHaveValue("in_review");
  const signed = await page.evaluate(
    () => window.__BUZZ_E2E_SIGNED_EVENTS__ ?? [],
  );
  const revision = signed.find((event) => event.kind === 40021);
  expect(revision).toBeTruthy();
  const content = JSON.parse(revision?.content ?? "{}") as {
    set?: { status?: string };
    reason?: string | null;
  };
  expect(content.set?.status).toBe("in_review");
  expect(content.reason).toBe("Needs owner review");
  await expect(page.getByTestId("dispatch-status-reason")).toHaveValue("");
});

test("detail emits a reason-less status change", async ({ page }) => {
  await openDispatch(page);
  await openPrimaryIssue(page);
  const status = page.getByLabel("Issue status");
  await status.selectOption("in_review");
  await expect(page.getByText("Issue updated")).toBeVisible();
  await expect(status).toHaveValue("in_review");
  const signed = await page.evaluate(
    () => window.__BUZZ_E2E_SIGNED_EVENTS__ ?? [],
  );
  const revision = signed.find((event) => event.kind === 40021);
  expect(revision).toBeTruthy();
  const content = JSON.parse(revision?.content ?? "{}") as {
    set?: { status?: string };
    reason?: string | null;
  };
  expect(content.set?.status).toBe("in_review");
  expect(content.reason).toBeNull();
});

test("detail does not keep an optimistic status when the fold rejects the revision", async ({
  page,
}) => {
  await openDispatch(page, context(), { rejectNextRevision: true });
  await openPrimaryIssue(page);
  await page.getByTestId("dispatch-status-reason").fill("Execution started");
  const status = page.getByLabel("Issue status");
  await status.selectOption("in_progress");
  await expect(page.getByText(/The update was not applied/)).toBeVisible();
  await expect(status).toHaveValue("triage");
  await expect(page.getByText("Issue updated")).toHaveCount(0);
  await expect(page.getByText("Triage → In Progress")).toHaveCount(0);
  const signed = await page.evaluate(
    () => window.__BUZZ_E2E_SIGNED_EVENTS__ ?? [],
  );
  expect(
    signed.some(
      (event) =>
        event.kind === 40021 &&
        JSON.parse(event.content).set.status === "in_progress",
    ),
  ).toBe(true);
  await expect(page.getByTestId("dispatch-status-reason")).toHaveValue(
    "Execution started",
  );
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
  await expect(page.getByText("Needs owner review")).toBeVisible();
  // Upstream #7188 gives cold profile batches three exponential-backoff
  // retries (1s + 2s + 4s) before exposing their terminal error state.
  await expect(
    page.getByText(/Some profile names are unavailable/),
  ).toBeVisible({ timeout: 12_000 });
  await expect(page.getByTestId("dispatch-comment")).toHaveCount(0);
});

function assignPrimary(assignee: string) {
  return context({
    issues: context().issues.map((entry) =>
      entry.issueId === PRIMARY_ISSUE ? { ...entry, assignee } : entry,
    ),
  });
}

test("assignee rail shows the profile name, not the pubkey", async ({
  page,
}) => {
  // OTHER is seeded in searchProfiles as kind:0 displayName "Helm".
  await openDispatch(page, assignPrimary(OTHER));
  await openPrimaryIssue(page);

  // Scoped to the name node: the row also renders an avatar whose fallback is
  // the first letter, so the button's full text is "HHelm".
  await expect(page.getByTestId("dispatch-assignee-name")).toHaveText("Helm");
  await expect(page.getByText(OTHER, { exact: true })).toHaveCount(0);
  // The pubkey field is reachable, but it is not the resting state.
  await expect(page.getByLabel("Issue assignee pubkey")).toHaveCount(0);

  await waitForAnimations(page);
  await page
    .locator("aside")
    .screenshot({ path: "test-results/dispatch-assignee/01-name.png" });
});

test("assignee rail falls back to a truncated pubkey with no profile", async ({
  page,
}) => {
  const unknown = "f".repeat(64);
  await openDispatch(page, assignPrimary(unknown));
  await openPrimaryIssue(page);

  await expect(page.getByTestId("dispatch-assignee-name")).toHaveText(
    `${"f".repeat(8)}…ffff`,
  );
  await expect(page.getByText(unknown, { exact: true })).toHaveCount(0);

  await waitForAnimations(page);
  await page
    .locator("aside")
    .screenshot({ path: "test-results/dispatch-assignee/02-fallback.png" });
});

async function readAssigneeRevision(page: import("@playwright/test").Page) {
  const signed = await page.evaluate(
    () => window.__BUZZ_E2E_SIGNED_EVENTS__ ?? [],
  );
  const revision = signed.findLast((event) => event.kind === 40021);
  return JSON.parse(revision?.content ?? "{}") as {
    set?: { assignee?: string | null };
    clear?: string[];
  };
}

/**
 * The structural fingerprint of a user-search popup: every class string that
 * comes from the shared shell, none of the per-call-site copy.
 *
 * Two popups that agree here are rendering the same component, not two
 * hand-matched copies of it — which is the property "looks exactly like the
 * members popup" actually needs, and the one a screenshot comparison cannot
 * distinguish from a lucky clone.
 */
async function popupShellSignature(
  page: import("@playwright/test").Page,
  testId: string,
) {
  return page.evaluate((id) => {
    const content = document.querySelector(`[data-testid="${id}"]`);
    if (!content) throw new Error(`popup ${id} is not mounted`);
    const searchShell = content.querySelector("label");
    return {
      content: content.className,
      header: content.children[0]?.className ?? null,
      closeButton: content.querySelector("button")?.className ?? null,
      searchShell: searchShell?.className ?? null,
      searchInput: searchShell?.querySelector("input")?.className ?? null,
      body: content.children[1]?.className ?? null,
      panel: content.querySelector("section > div")?.className ?? null,
      sectionTitle:
        content.querySelector("section > div > div")?.className ?? null,
    };
  }, testId);
}

test("assignee row opens the members-style picker and assigns from search", async ({
  page,
}) => {
  await openDispatch(page, assignPrimary(OTHER));
  await openPrimaryIssue(page);

  await page.getByTestId("dispatch-assignee").click();
  await expect(page.getByTestId("dispatch-assignee-picker")).toBeVisible();
  // The current assignee is not offered as something to assign again; it sits
  // in its own section with the action that is actually available on it.
  await expect(
    page.getByTestId(`dispatch-assignee-option-${OTHER}`).getByRole("button", {
      name: "Unassign",
    }),
  ).toBeVisible();

  await page.getByTestId("dispatch-assignee-search").fill("Tyler");
  const candidate = page.getByTestId(`dispatch-assignee-option-${SELF}`);
  await expect(candidate).toBeVisible();
  await candidate.getByRole("button", { name: "Assign" }).click();

  await expect(page.getByTestId("dispatch-assignee-picker")).toHaveCount(0);
  await expect(page.getByText("Issue updated")).toBeVisible();
  expect((await readAssigneeRevision(page)).set?.assignee).toBe(SELF);
});

test("assignee picker unassigns from the assigned section", async ({
  page,
}) => {
  await openDispatch(page, assignPrimary(OTHER));
  await openPrimaryIssue(page);

  await page.getByTestId("dispatch-assignee").click();
  await page
    .getByTestId(`dispatch-assignee-option-${OTHER}`)
    .getByRole("button", { name: "Unassign" })
    .click();

  await expect(page.getByText("Issue updated")).toBeVisible();
  // Removing an assignee is a `clear`, not a `set` to null — see
  // `dispatchRevisionFields` in mutations.ts.
  const revision = await readAssigneeRevision(page);
  expect(revision.clear).toEqual(["assignee"]);
  expect(revision.set?.assignee).toBeUndefined();
  await expect(page.getByTestId("dispatch-assignee-name")).toHaveText(
    "Unassigned",
  );
});

test("assignee picker still assigns a pasted key the directory cannot find", async ({
  page,
}) => {
  // The directory is a full-text search over kind:0 content, where a pubkey
  // never appears — so this key is unreachable by search and only assignable
  // because the picker offers a parsed key as its own candidate.
  const unpublished = "c".repeat(64);
  await openDispatch(page);
  await openPrimaryIssue(page);

  await page.getByTestId("dispatch-assignee").click();
  await page.getByTestId("dispatch-assignee-search").fill(unpublished);
  const pasted = page.getByTestId(`dispatch-assignee-option-${unpublished}`);
  await expect(pasted).toBeVisible();
  await pasted.getByRole("button", { name: "Assign" }).click();

  await expect(page.getByText("Issue updated")).toBeVisible();
  expect((await readAssigneeRevision(page)).set?.assignee).toBe(unpublished);
});

test("assignee picker prefers current colleagues, collapses stale names, and hides blanks", async ({
  page,
}) => {
  const staleHelmA = "a".repeat(64);
  const staleHelmB = "b".repeat(64);
  const blank = "c".repeat(64);
  const staleUniqueA = "d".repeat(64);
  const staleUniqueB = "e".repeat(64);
  await installMockBridge(page, {
    dispatch: {
      workspace: { id: WORKSPACE, name: "Dispatch Test Workspace" },
      context: context(),
    },
    searchProfiles: [
      { pubkey: OTHER, displayName: "Helm" },
      { pubkey: staleHelmA, displayName: "Helm" },
      { pubkey: staleHelmB, displayName: "Helm" },
      { pubkey: blank, displayName: "  " },
      { pubkey: staleUniqueA, displayName: "Retired Agent" },
      { pubkey: staleUniqueB, displayName: "Retired Agent" },
    ],
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-dispatch-view").click();
  await openPrimaryIssue(page);
  await page.getByTestId("dispatch-assignee").click();

  await expect(
    page.getByTestId(`dispatch-assignee-option-${OTHER}`),
  ).toBeVisible();
  await expect(
    page.getByTestId(`dispatch-assignee-option-${staleHelmA}`),
  ).toHaveCount(0);
  await expect(
    page.getByTestId(`dispatch-assignee-option-${staleHelmB}`),
  ).toHaveCount(0);
  await expect(
    page.getByTestId(`dispatch-assignee-option-${blank}`),
  ).toHaveCount(0);
  await expect(
    page.getByTestId(`dispatch-assignee-option-${staleUniqueA}`),
  ).toBeVisible();
  await expect(
    page.getByTestId(`dispatch-assignee-option-${staleUniqueB}`),
  ).toHaveCount(0);
});

test("assignee picker and the channel members popup are the same shell", async ({
  page,
}) => {
  await openDispatch(page, assignPrimary(OTHER));

  await page.getByTestId("channel-general").click();
  await page.getByTestId("channel-members-trigger").click();
  await expect(page.getByTestId("members-sidebar")).toBeVisible();
  const membersSignature = await popupShellSignature(page, "members-sidebar");
  await waitForAnimations(page);
  await page
    .getByTestId("members-sidebar")
    .screenshot({ path: "test-results/dispatch-assignee/03-members.png" });
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("members-sidebar")).toHaveCount(0);

  await page.getByTestId("open-dispatch-view").click();
  await openPrimaryIssue(page);
  await page.getByTestId("dispatch-assignee").click();
  await expect(page.getByTestId("dispatch-assignee-picker")).toBeVisible();
  const assigneeSignature = await popupShellSignature(
    page,
    "dispatch-assignee-picker",
  );
  await waitForAnimations(page);
  await page
    .getByTestId("dispatch-assignee-picker")
    .screenshot({ path: "test-results/dispatch-assignee/04-assignee.png" });

  expect(assigneeSignature).toEqual(membersSignature);
  // Guard the guard: a signature of all-nulls would compare equal and prove
  // nothing.
  expect(Object.values(membersSignature).every(Boolean)).toBe(true);
});

/** A relay-hosted avatar — the shape `rewriteRelayUrl` has to intercept. */
function relayAvatarUrl(hexDigit: string) {
  return `http://localhost:3000/media/${hexDigit.repeat(64)}.png`;
}

const PROXIED_AVATAR = /^http:\/\/127\.0\.0\.1:54321\/media\/[\da-f]{64}\.png$/;

/** Relative luminance of a `rgb(r, g, b)` string, 0 (black) to 1 (white). */
function luminanceOf(color: string) {
  const [r, g, b] = (/rgba?\((\d+), (\d+), (\d+)/.exec(color) ?? [])
    .slice(1)
    .map(Number);
  return (0.299 * r + 0.587 * g + 0.114 * b) / 255;
}

test("Dispatch avatars load through the media proxy and take the app theme", async ({
  page,
}) => {
  // Our avatars are relay-hosted, and bare relay media URLs are unauthenticated
  // 401s — every avatar has to go through the local proxy that `UserAvatar`
  // applies. Three Dispatch surfaces used to pass `profile.avatarUrl` straight
  // to an `<img>`, so an identity with a published photo rendered as its
  // initial. This pins the proxy path on the assignee rail and the picker: with
  // a raw `<img src>` the src is the relay URL and this fails.
  await page.route(`http://127.0.0.1:54321/media/**`, (route) =>
    route.fulfill({
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#22c55e"/></svg>',
      contentType: "image/svg+xml",
    }),
  );
  await page.addInitScript(() => {
    window.localStorage.setItem("buzz-theme", "buzz-dark");
  });
  await installMockBridge(page, {
    dispatch: {
      workspace: { id: WORKSPACE, name: "Dispatch Test Workspace" },
      context: assignPrimary(OTHER),
    },
    searchProfiles: [
      { pubkey: OTHER, displayName: "Helm", avatarUrl: relayAvatarUrl("e") },
      { pubkey: SELF, displayName: "Tyler", avatarUrl: relayAvatarUrl("d") },
    ],
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-dispatch-view").click();
  await openPrimaryIssue(page);

  const railAvatar = page.getByTestId("dispatch-assignee").locator("img");
  await expect(railAvatar).toHaveAttribute("src", PROXIED_AVATAR);
  // A broken `<img>` still renders "an avatar" — assert the pixels decoded.
  await expect
    .poll(() =>
      railAvatar.evaluate((img: HTMLImageElement) => img.naturalWidth),
    )
    .toBeGreaterThan(0);

  await page.getByTestId("dispatch-assignee").click();
  await expect(page.getByTestId("dispatch-assignee-picker")).toBeVisible();
  const rowAvatar = page
    .getByTestId(`dispatch-assignee-option-${SELF}`)
    .locator("img");
  await expect(rowAvatar).toHaveAttribute("src", PROXIED_AVATAR);
  await expect
    .poll(() => rowAvatar.evaluate((img: HTMLImageElement) => img.naturalWidth))
    .toBeGreaterThan(0);

  // The popup surface is a theme token, not a hardcoded light panel. The exact
  // palette value is deliberately not pinned — only that a dark theme renders
  // dark, which is what a literal `bg-white` would break.
  const background = await page
    .getByTestId("dispatch-assignee-picker")
    .evaluate((element) => getComputedStyle(element).backgroundColor);
  expect(luminanceOf(background)).toBeLessThan(0.5);
});

test("detail never turns a non-http provenance source into a link", async ({
  page,
}) => {
  const unsafe = context({
    activity: context().activity.map((entry, index) =>
      index === 0 && entry.import
        ? {
            ...entry,
            import: { ...entry.import, source_url: "javascript:alert(1)" },
          }
        : entry,
    ),
  });
  await openDispatch(page, unsafe);
  await openPrimaryIssue(page);
  await expect(page.getByText("Imported from Linear")).toBeVisible();
  await expect(
    page.getByRole("link", { name: /Imported from Linear/ }),
  ).toHaveCount(0);
});

test("human editor saves one complete revision and reads it back", async ({
  page,
}) => {
  await openDispatch(page);
  await openPrimaryIssue(page);
  await page.getByRole("button", { name: "Edit issue", exact: true }).click();
  const editor = page.getByRole("dialog");
  await editor
    .getByLabel("Title", { exact: true })
    .fill("A clear human outcome");
  await editor
    .getByLabel("Description", { exact: true })
    .fill("## Acceptance\n\nA person can understand this work.");
  await editor.getByLabel("Due date", { exact: true }).fill("2026-09-30");
  await editor.getByLabel(/Estimate/).fill("45");
  await editor
    .getByLabel("Labels", { exact: true })
    .fill("customer, usability");
  await editor.getByRole("button", { name: "Preview", exact: true }).click();
  await expect(
    editor.getByRole("heading", { name: "Acceptance", exact: true }),
  ).toBeVisible();
  await editor
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect(editor).toHaveCount(0);
  await expect(
    page.getByRole("heading", { name: "A clear human outcome", exact: true }),
  ).toBeVisible();
  const revisions = await page.evaluate(() =>
    (window.__BUZZ_E2E_SIGNED_EVENTS__ ?? []).filter(
      (event) => event.kind === 40021,
    ),
  );
  expect(revisions).toHaveLength(1);
  const wire = JSON.parse(revisions[0].content);
  expect(wire.set.title).toBe("A clear human outcome");
  expect(wire.set.estimate_minutes).toBe(45);
  expect(wire.set.due.precision).toBe("date");
  await page.getByRole("button", { name: "Edit issue", exact: true }).click();
  await expect(
    page.getByRole("dialog").getByLabel("Title", { exact: true }),
  ).toHaveValue("A clear human outcome");
  await expect(
    page.getByText("Keep my draft on the latest version"),
  ).toHaveCount(0);
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Close", exact: true })
    .first()
    .click();
  await page.screenshot({
    path: "/Users/titusclaw/.buzz/RESEARCH/DISPATCH_ASSESSMENT_2026_09_10/implementation-detail.png",
  });
});

test("rejected human edit keeps the draft and exposes recovery", async ({
  page,
}) => {
  await openDispatch(page, context(), { rejectNextRevision: true });
  await openPrimaryIssue(page);
  await page.getByRole("button", { name: "Edit issue", exact: true }).click();
  const editor = page.getByRole("dialog");
  await editor.getByLabel("Title", { exact: true }).fill("Preserve this draft");
  await editor
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect(editor.getByRole("alert")).toBeVisible();
  await expect(editor.getByLabel("Title", { exact: true })).toHaveValue(
    "Preserve this draft",
  );
  await expect(
    editor.getByRole("button", { name: "Save changes", exact: true }),
  ).toBeEnabled();
});

test("shared decision records preserve the named choice and the issue status", async ({
  page,
}) => {
  await openDispatch(page);
  await openPrimaryIssue(page);
  await page.getByRole("button", { name: "Ask for a decision" }).click();
  const dialog = page.getByRole("dialog", { name: "Ask for a decision" });
  await dialog
    .getByLabel("Question", { exact: true })
    .fill("Should we release this version?");
  await dialog
    .getByLabel("Consequence", { exact: true })
    .nth(0)
    .fill("Install after acceptance");
  await dialog
    .getByLabel("Consequence", { exact: true })
    .nth(1)
    .fill("Keep the current version");
  await dialog.getByRole("button", { name: "Create decision" }).click();
  await expect(dialog).not.toBeVisible();
  await page.getByRole("button", { name: "Choose Hold", exact: true }).click();
  await page
    .getByLabel("Reason for this decision")
    .fill("We need an installed acceptance check.");
  await page
    .getByRole("button", { name: "Record decision", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Record decision", exact: true }),
  ).toHaveCount(0);
  await expect(
    page
      .getByTestId("dispatch-decision-resolution")
      .getByText("We need an installed acceptance check.", { exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel("Issue status")).toHaveValue("triage");
  const events = await page.evaluate(
    () => window.__BUZZ_E2E_PUBLISHED_DISPATCH_EVENTS__ ?? [],
  );
  const records = events
    .filter((event) => event.kind === 40026)
    .map((event) => JSON.parse(event.content));
  expect(records).toHaveLength(2);
  expect(records[1].parent_event).toBe(
    events.find((event) => event.kind === 40026)?.id,
  );
  expect(
    records[1].payload.options.find(
      (option: { id: string }) =>
        option.id === records[1].payload.selected_option,
    ).label,
  ).toBe("Hold");
  await page.getByTestId("dispatch-detail-back").click();
  await page.getByLabel("Attention view").selectOption("decision");
  await expect(page.getByTestId("dispatch-list-row")).toHaveCount(0);
});

test("lost shared-save acknowledgement recovers the accepted event without a second publication", async ({
  page,
}) => {
  await openDispatch(page);
  await page.getByRole("button", { name: "Save or manage views" }).click();
  const dialog = page.getByRole("dialog", { name: "Saved views" });
  await dialog.getByLabel("View name").fill("Team focus");
  await dialog.getByLabel("Share with this workspace").check();
  await page.evaluate(() => {
    window.__BUZZ_E2E_FAIL_DISPATCH_ACKS__ = 1;
  });
  await dialog
    .getByRole("button", { name: "Save current view", exact: true })
    .click();
  await expect(dialog.getByRole("alert")).toBeVisible();
  await dialog
    .getByRole("button", { name: "Save current view", exact: true })
    .click();
  await expect(dialog).not.toBeVisible();
  await expect(
    page.getByLabel("Saved view").getByRole("option", { name: "Team focus" }),
  ).toHaveCount(1);
  expect(
    await page.evaluate(() =>
      (window.__BUZZ_E2E_PUBLISHED_DISPATCH_EVENTS__ ?? []).filter(
        (event) => event.kind === 40026,
      ),
    ),
  ).toHaveLength(1);
});

test("project outcome and milestone survive navigation through linked issues", async ({
  page,
}) => {
  const fixture = context();
  const project = "10000000-0000-4000-8000-000000000001";
  fixture.projects[0].projectId = project;
  for (const issue of fixture.issues) issue.projectId = project;
  await openDispatch(page, fixture);
  await page
    .getByRole("navigation", { name: "Dispatch views" })
    .getByRole("button", { name: "Projects", exact: true })
    .click();
  await page
    .getByTestId("dispatch-projects")
    .getByRole("button", { name: /Dispatch Desktop/ })
    .click();
  await page.getByRole("button", { name: "Edit project plan" }).click();
  const dialog = page.getByRole("dialog");
  await dialog
    .getByLabel("Target outcome", { exact: true })
    .fill("Make human handoffs dependable");
  await dialog.getByRole("button", { name: "Add milestone" }).click();
  await dialog
    .getByLabel("Milestone 1", { exact: true })
    .fill("Installed acceptance");
  await dialog.getByText(/Contributing issues/).click();
  await dialog.getByRole("checkbox", { name: /P0 oldest canonical/ }).check();
  await dialog.getByRole("button", { name: "Save project plan" }).click();
  await expect(dialog).not.toBeVisible();
  await expect(page.getByText("Make human handoffs dependable")).toBeVisible();
  await expect(
    page.getByRole("progressbar", { name: "Installed acceptance progress" }),
  ).toHaveAttribute("max", "1");
  await page
    .getByRole("button", { name: /D-AAAAAA · P0 oldest canonical/ })
    .first()
    .click();
  await expect(page.getByTestId("dispatch-issue-detail")).toBeVisible();
  await page.getByTestId("dispatch-detail-back").click();
  await expect(page.getByTestId("dispatch-project-home")).toBeVisible();
  await expect(page.getByText("Make human handoffs dependable")).toBeVisible();
});

test("bulk edit previews two issues and confirms each change", async ({
  page,
}) => {
  await openDispatch(page);
  await page.getByTestId("dispatch-tab-list").click();
  await page
    .getByRole("checkbox", {
      name: "Select P0 oldest canonical triage issue",
      exact: true,
    })
    .check();
  await page
    .getByRole("checkbox", {
      name: "Select P1 older than second P0",
      exact: true,
    })
    .check();
  await page.getByRole("button", { name: "Edit selected issues" }).click();
  const dialog = page.getByRole("dialog", { name: "Edit 2 selected issues" });
  await dialog.getByLabel("Status", { exact: true }).selectOption("todo");
  await dialog.getByRole("button", { name: "Apply reviewed changes" }).click();
  await expect(dialog.getByText("2 of 2 confirmed")).toBeVisible();
  const changes = await page.evaluate(() =>
    (window.__BUZZ_E2E_PUBLISHED_DISPATCH_EVENTS__ ?? [])
      .filter((event) => event.kind === 40021)
      .map((event) => JSON.parse(event.content)),
  );
  expect(changes).toHaveLength(2);
  expect(new Set(changes.map((change) => change.issue_id)).size).toBe(2);
  expect(changes.every((change) => change.set.status === "todo")).toBe(true);
});

test("import activation is explicit and delivery evidence does not close or verify work", async ({
  page,
}) => {
  await openDispatch(page);
  await openPrimaryIssue(page);
  await expect(page.getByText("Imported history · parked")).toBeVisible();
  await page.getByRole("button", { name: "Accept into current work" }).click();
  await page
    .getByLabel("Reason", { exact: true })
    .fill("Accepted for the current release");
  await page.getByRole("button", { name: "Confirm activation change" }).click();
  await expect(page.getByText("Imported work · activated")).toBeVisible();
  await page.getByRole("button", { name: "Record evidence" }).click();
  const dialog = page.getByRole("dialog", { name: "Record delivery evidence" });
  await dialog
    .getByLabel("Artifact, commit, report or environment")
    .fill("commit:abc123 / test report");
  await dialog
    .getByLabel("What was checked and what happened")
    .fill("Ran the behavior check; installation is still pending.");
  await dialog.getByRole("button", { name: "Save observation" }).click();
  await expect(dialog).not.toBeVisible();
  await expect(
    page.getByRole("region", { name: "Delivery evidence" }),
  ).toContainText("Tests · Passed / confirmed");
  await expect(page.getByLabel("Issue status")).toHaveValue("triage");
  await expect(page.getByTestId("dispatch-task-attempt")).toContainText(
    "Working",
  );
});

test("a message can create linked work without copying its source text", async ({
  page,
}) => {
  await openDispatch(page);
  await page.getByTestId("channel-general").click();
  const id = "f".repeat(64);
  await page.evaluate(
    (id) =>
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: "Sensitive source detail stays in the conversation",
        id,
      }),
    id,
  );
  const row = page.locator(`[data-message-id="${id}"]`).first();
  await expect(row).toContainText("Sensitive source detail");
  await row.hover();
  await row.getByTestId(`more-actions-${id}`).click();
  await page.getByTestId(`dispatch-message-${id}`).click();
  const dialog = page.getByTestId("conversation-dispatch-dialog");
  await expect(dialog).toBeVisible();
  await dialog
    .getByLabel("Outcome or next step")
    .fill("Investigate the customer concern");
  await dialog.getByRole("button", { name: "Create linked issue" }).click();
  await expect(
    dialog.getByRole("button", { name: "Open issue" }),
  ).toBeVisible();
  const roots = await page.evaluate(() =>
    (window.__BUZZ_E2E_PUBLISHED_DISPATCH_EVENTS__ ?? [])
      .filter((event) => event.kind === 40020)
      .map((event) => JSON.parse(event.content)),
  );
  expect(roots).toHaveLength(1);
  expect(roots[0].source_event_id).toBe(id);
  expect(roots[0].body).not.toContain("Sensitive source detail");
  expect(roots[0].external_refs[0].ref).toContain(id);
  await dialog.getByRole("button", { name: "Open issue" }).click();
  await expect(
    page.getByRole("heading", {
      name: "Investigate the customer concern",
      exact: true,
    }),
  ).toBeVisible();
});

test("automation stays in preview until a configured service acknowledges it", async ({
  page,
}) => {
  const fixture = automationFixture({
    notify_triage: true,
    issue_ids: [AUTOMATION_ISSUE],
  });
  fixture.automation = { actions: [], heartbeats: [] };
  await openAutomation(page, fixture);
  const card = page.getByTestId("automation-run-card");
  await expect(card.getByRole("status")).toHaveText("Waiting for connection");
  await expect(
    card.getByRole("button", { name: "Turn on…", exact: true }),
  ).toBeDisabled();
  await card.getByRole("button", { name: "Pause", exact: true }).click();
  await expect(
    card.getByRole("button", { name: "Check before turning on", exact: true }),
  ).toBeVisible();
  await expect(card.getByRole("status")).toHaveText("Waiting for connection");
  const records = await publishedRecords(page);
  expect(records).toHaveLength(1);
  expect(records[0].payload.controls.mode).toBe("paused");
});

test("human assignment confirms its issue link before notifying the executor and retains its owner", async ({
  page,
}) => {
  const fixture = context({ linkedTasks: [] });
  const primary = fixture.issues.find(
    (issue) => issue.issueId === PRIMARY_ISSUE,
  );
  if (!primary) throw new Error("Primary fixture issue is missing");
  primary.assignee = SELF;
  await openDispatch(page, fixture, {
    members: [SELF, OTHER],
    assignmentAckDelayMs: 2500,
  });
  await openPrimaryIssue(page);
  await page
    .getByRole("button", { name: "Assign execution", exact: true })
    .click();
  const dialog = page.getByRole("dialog", {
    name: "Assign execution",
    exact: true,
  });
  await dialog
    .getByRole("button", { name: "Choose a person", exact: true })
    .click();
  const picker = page.getByTestId("dispatch-assignee-picker");
  await page.getByTestId("dispatch-assignee-search").fill("Helm");
  await picker
    .getByTestId(`dispatch-assignee-option-${OTHER}`)
    .getByRole("button", { name: "Assign", exact: true })
    .click();
  await dialog
    .getByLabel("Reason to proceed with this human override")
    .fill("Explicit small test; the human owner will verify the result.");
  await dialog
    .getByRole("button", {
      name: "Confirm assignment and notify executor",
      exact: true,
    })
    .click();
  // Hold the assignment acknowledgement after the issue link is accepted.
  // That expected link update must not appear as an unrelated stale edit.
  await expect
    .poll(
      () =>
        page.evaluate(() =>
          (window.__BUZZ_E2E_PUBLISHED_DISPATCH_EVENTS__ ?? []).some(
            (event) => event.kind === 9,
          ),
        ),
      { timeout: 15000 },
    )
    .toBe(true);
  expect(
    await dialog
      .getByText("The issue changed during this preview.", { exact: false })
      .count(),
  ).toBe(0);
  // The transaction reads current authority, links the issue, then confirms
  // accepted task history; allow those reads plus the delayed acknowledgement.
  await expect(dialog).not.toBeVisible({ timeout: 15000 });
  await expect(
    page.getByText(
      "Assignment confirmed in accepted task history. The issue owner is unchanged.",
      { exact: true },
    ),
  ).toBeVisible();
  const events = await page.evaluate(
    () => window.__BUZZ_E2E_PUBLISHED_DISPATCH_EVENTS__ ?? [],
  );
  const linkIndex = events.findIndex((event) => event.kind === 40021),
    assignmentIndex = events.findIndex((event) => event.kind === 9);
  expect(linkIndex).toBeGreaterThanOrEqual(0);
  expect(assignmentIndex).toBeGreaterThan(linkIndex);
  expect(events.filter((event) => event.kind === 9)).toHaveLength(1);
  const revision = JSON.parse(events[linkIndex].content);
  expect(revision.set.assignee).toBeUndefined();
  expect(
    revision.external_refs.add.some(
      (ref: { type: string; ref: string }) =>
        ref.type === "task_event" && ref.ref === events[assignmentIndex].id,
    ),
  ).toBe(true);
  await waitForAnimations(page);
  await page.screenshot({
    path: "/Users/titusclaw/.buzz/RESEARCH/DISPATCH_ASSESSMENT_2026_09_10/implementation-assignment.png",
  });
});

test("independent human review binds the reported result and never closes its issue", async ({
  page,
}) => {
  const fixture = context({
    linkedTasks: [
      {
        issueId: PRIMARY_ISSUE,
        taskId: "dddddddd-dddd-4ddd-8ddd-dddddddddddd",
        title: "Completed scoped attempt",
        status: "done",
        assignee: OTHER,
        assigner: "deadbeef".repeat(8),
        reviewer: "deadbeef".repeat(8),
        assignmentEventId: "a".repeat(64),
        eventId: "b".repeat(64),
        channelId: WORKSPACE,
        createdAt: 1720000300,
        dispatchRequestId: null,
        verification: {
          method: "Inspect the delivered artifact",
          evidence_ref: "https://example.test/result",
        },
        history: [],
      },
    ],
  });
  await openDispatch(page, fixture, { members: [SELF, OTHER] });
  await openPrimaryIssue(page);
  await page
    .getByRole("navigation", { name: "Issue sections" })
    .getByRole("button", { name: "Execution", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Review reported result", exact: true })
    .click();
  const dialog = page.getByRole("dialog", {
    name: "Review the reported result",
  });
  await expect(
    dialog.getByRole("button", { name: "Accept reviewed result", exact: true }),
  ).toBeDisabled();
  await dialog
    .getByLabel("Review evidence", { exact: true })
    .fill("https://example.test/independent-review");
  await dialog
    .getByLabel("What did you check?", { exact: true })
    .fill(
      "Checked the artifact and its negative control against the agreed scope.",
    );
  await dialog
    .getByRole("button", { name: "Accept reviewed result", exact: true })
    .click();
  await expect(dialog).not.toBeVisible();
  await expect(page.getByTestId("dispatch-task-attempt")).toContainText(
    "Independently verified",
  );
  const events = await page.evaluate(
    () => window.__BUZZ_E2E_PUBLISHED_DISPATCH_EVENTS__ ?? [],
  );
  expect(events.filter((event) => event.kind === 40021)).toHaveLength(0);
  const review = events.find((event) => event.kind === 9);
  expect(review).toBeTruthy();
  expect(review?.content).toContain(
    '"previous_event_id":"' + "b".repeat(64) + '"',
  );
  expect(review?.tags.some((tag) => tag[0] === "p")).toBe(false);
});

test("conversation updates recover a lost acknowledgement and deduplicate repeat shares", async ({
  page,
}) => {
  await openDispatch(page, context(), { members: [SELF, OTHER] });
  await openPrimaryIssue(page);
  await page.evaluate(() => {
    window.__BUZZ_E2E_FAIL_DISPATCH_ACKS__ = 1;
  });
  for (let attempt = 0; attempt < 2; attempt++) {
    await page
      .getByRole("button", { name: "Share an update", exact: true })
      .click();
    const dialog = page.getByRole("dialog");
    await expect(
      dialog.getByRole("textbox", { name: "Message", exact: true }),
    ).toHaveValue(/Current issue status/);
    await dialog
      .getByRole("button", { name: "Share reviewed update", exact: true })
      .click();
    await expect(dialog).not.toBeVisible();
    await expect(
      page.getByText("Shared in the conversation.", { exact: false }),
    ).toBeVisible();
  }
  const events = await page.evaluate(
    () => window.__BUZZ_E2E_PUBLISHED_DISPATCH_EVENTS__ ?? [],
  );
  expect(
    events.filter(
      (event) =>
        event.kind === 9 &&
        event.tags.some(
          (tag) => tag[0] === "t" && tag[1] === "dispatch-update",
        ),
    ),
  ).toHaveLength(1);
  expect(events.some((event) => event.kind === 40021)).toBe(false);
});

test("a rejected issue edit can become a durable draft without changing accepted work", async ({
  page,
}) => {
  await openDispatch(page, context(), { rejectNextRevision: true });
  await openPrimaryIssue(page);
  await page.getByRole("button", { name: "Edit issue", exact: true }).click();
  const editor = page.getByRole("dialog");
  await editor
    .getByLabel("Title", { exact: true })
    .fill("Retain the rejected edit");
  await editor
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect(editor.getByRole("alert")).toBeVisible();
  await editor
    .getByRole("button", { name: "Close and keep draft", exact: true })
    .click();
  await page.getByText(/issue change\(s\) awaiting confirmation/).click();
  await page
    .getByRole("button", { name: "Keep as draft", exact: true })
    .click();
  await page
    .getByText("Saved drafts on this device (1)", { exact: true })
    .click();
  await expect(page.getByText(/title: Retain the rejected edit/)).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Copy draft", exact: true }),
  ).toBeVisible();
  const events = await page.evaluate(
    () => window.__BUZZ_E2E_PUBLISHED_DISPATCH_EVENTS__ ?? [],
  );
  expect(events.filter((event) => event.kind === 40021)).toHaveLength(1);
  await page.reload({ waitUntil: "domcontentloaded" });
  await expect(
    page.getByText("Saved drafts on this device (1)", { exact: true }),
  ).toBeVisible();
});
