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
  dispatchOptions: { rejectNextRevision?: boolean } = {},
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
  await page.getByRole("button", { name: /^Triage(?: \d+)?$/ }).click();
  await page
    .getByTestId("dispatch-issue-11111111-1111-4111-8111-111111111111")
    .click();
  await expect(page.getByTestId("dispatch-issue-detail")).toBeVisible();
}

test("triage renders unassigned and assigned current-fold sections in P0-first then-oldest order", async ({
  page,
}) => {
  await openDispatch(page);
  await page.getByRole("button", { name: /^Triage(?: \d+)?$/ }).click();
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
  await expect(
    page.getByText(/Canonical fold rejected the revision/),
  ).toBeVisible();
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
  await expect(
    page.getByText(/Some profile names are unavailable/),
  ).toBeVisible();
  await expect(page.getByText("Needs owner review")).toBeVisible();
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
  expect(
    await railAvatar.evaluate((img: HTMLImageElement) => img.naturalWidth),
  ).toBeGreaterThan(0);

  await page.getByTestId("dispatch-assignee").click();
  await expect(page.getByTestId("dispatch-assignee-picker")).toBeVisible();
  const rowAvatar = page
    .getByTestId(`dispatch-assignee-option-${SELF}`)
    .locator("img");
  await expect(rowAvatar).toHaveAttribute("src", PROXIED_AVATAR);
  expect(
    await rowAvatar.evaluate((img: HTMLImageElement) => img.naturalWidth),
  ).toBeGreaterThan(0);

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
