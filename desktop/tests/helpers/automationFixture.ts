import { expect, type Page } from "@playwright/test";
import type {
  DispatchContext,
  DispatchIssue,
} from "../../src/features/dispatch/types";
import type {
  AutomationControls,
  DispatchRecordPayload,
  DispatchRecordProjection,
} from "../../src/features/dispatch/recordTypes";
import { TEST_IDENTITIES, installMockBridge } from "./bridge";

export const WORKSPACE = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
export const SERVICE = "b".repeat(64);
export const AGENT = TEST_IDENTITIES.alice.pubkey;
export const OWNER = TEST_IDENTITIES.tyler.pubkey;
export const ISSUE = "11111111-1111-4111-8111-111111111111";
export function planningRecord(
  payload: DispatchRecordPayload,
  digit = "a",
): DispatchRecordProjection {
  return {
    creator: OWNER,
    author: OWNER,
    event_id: digit.repeat(64),
    root_event: digit.repeat(64),
    created_at: Math.floor(Date.now() / 1000) - 60,
    record: {
      v: 1,
      record_id: `${digit.repeat(8)}-${digit.repeat(4)}-4${digit.repeat(3)}-8${digit.repeat(3)}-${digit.repeat(12)}`,
      revision: 0,
      parent_event: null,
      request_id: crypto.randomUUID(),
      archived: false,
      payload,
    },
  };
}
export function automationFixture(
  overrides: Partial<AutomationControls> = {},
): DispatchContext {
  const now = Math.floor(Date.now() / 1000);
  const controls: AutomationControls = {
    service_id: "workspace-automation",
    service_pubkey: SERVICE,
    mode: "preview",
    expires_at: null,
    max_actions_total: 3,
    max_actions_per_tick: 1,
    issue_ids: [],
    capture_channels: [],
    routing: [],
    notify_triage: false,
    unassigned_after_secs: null,
    notification_channel: WORKSPACE,
    ...overrides,
  };
  const record = planningRecord({ type: "automation", controls });
  const issue: DispatchIssue = {
    streamId: WORKSPACE,
    issueId: ISSUE,
    rootEvent: "1".repeat(64),
    rootCreatedAt: now - 90,
    tipEvent: "1".repeat(64),
    tipCreatedAt: now - 90,
    revision: 0,
    enteredStatusAt: now - 90,
    identifier: "D-111111",
    title: "Research a simpler onboarding flow",
    body: "Find a clearer first step for new members.",
    status: "todo",
    priority: "P2",
    assignee: OWNER,
    projectId: null,
    due: null,
    estimateMinutes: null,
    homeChannelId: WORKSPACE,
    workflow: "default",
    labels: ["research"],
    externalRefs: [],
    relations: [],
  };
  return {
    issues: [issue],
    activity: [],
    comments: [],
    linkedTasks: [],
    projects: [],
    diagnostics: [],
    taskDiagnostics: [],
    viewerRole: "maintainer",
    sectionErrors: {
      records: null,
      comments: null,
      projects: null,
      tasks: null,
    },
    snapshotAt: new Date().toISOString(),
    records: [record],
    admission: { sealed: [], legacyBaselines: 0, unsealed: 0, diagnostics: [] },
    automation: {
      actions: [],
      heartbeats: [
        {
          eventId: "c".repeat(64),
          signer: SERVICE,
          createdAt: now,
          serviceId: controls.service_id,
          observedAt: now,
          health: {
            status: "healthy",
            detail: JSON.stringify({
              approval: record.event_id,
              controls: controls.mode,
              spent: 0,
              pending: 0,
            }),
          },
        },
      ],
    },
  };
}
export async function openAutomation(
  page: Page,
  context = automationFixture(),
  options: { available?: boolean; enabled?: boolean } = {},
) {
  await installMockBridge(
    page,
    {
      dispatch: { workspace: { id: WORKSPACE, name: "Product work" }, context },
      searchProfiles: [
        { pubkey: OWNER, displayName: "Tyler" },
        { pubkey: AGENT, displayName: "Research helper", isAgent: true },
        { pubkey: SERVICE, displayName: "Workspace automation", isAgent: true },
      ],
    },
    { seedPreviewFeatures: options.enabled ?? true },
  );
  await page.goto("/", { waitUntil: "domcontentloaded" });
  if (options.enabled !== false) {
    await page.getByTestId("open-dispatch-view").click();
    await expect(page.getByTestId("dispatch-new-issue")).toBeEnabled();
  }
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  if (options.available === false) {
    await expect(page.getByTestId("settings-nav-dispatch-service")).toHaveCount(
      0,
    );
    return;
  }
  await page.getByTestId("settings-nav-dispatch-service").click();
  await expect(page.getByTestId("dispatch-automation")).toBeVisible();
}
export async function publishedRecords(page: Page) {
  return page.evaluate(() =>
    (window.__BUZZ_E2E_PUBLISHED_DISPATCH_EVENTS__ ?? [])
      .filter((event) => event.kind === 40026)
      .map((event) => JSON.parse(event.content)),
  );
}
