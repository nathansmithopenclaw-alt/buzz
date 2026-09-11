import { expect, test } from "@playwright/test";
import type {
  DispatchContext,
  DispatchIssue,
} from "../../src/features/dispatch/types";
import { installMockBridge } from "../helpers/bridge";

const workspace = "11111111-1111-4111-8111-111111111111";
const issue: DispatchIssue = {
  streamId: workspace,
  issueId: "22222222-2222-4222-8222-222222222222",
  rootEvent: "1".repeat(64),
  rootCreatedAt: 100,
  tipEvent: "1".repeat(64),
  tipCreatedAt: 100,
  revision: 0,
  enteredStatusAt: 100,
  identifier: "D-CACHED",
  title: "Saved issue ready to read",
  body: "Previously synchronized issue",
  status: "todo",
  priority: "P2",
  assignee: null,
  projectId: null,
  due: null,
  estimateMinutes: null,
  homeChannelId: null,
  workflow: "default",
  labels: [],
  externalRefs: [],
  relations: [],
};
const preview: DispatchContext = {
  issues: [issue],
  activity: [],
  comments: [],
  linkedTasks: [],
  projects: [],
  diagnostics: [],
  taskDiagnostics: [],
  viewerRole: "none",
  sectionErrors: { comments: null, projects: null, tasks: null },
  snapshotAt: new Date(100000).toISOString(),
};

test("saved issues paint before slow synchronization and then become current", async ({
  page,
}) => {
  await page.addInitScript(
    ({ workspace }) =>
      localStorage.setItem("buzz-dispatch-last-workspace-v1", workspace),
    { workspace },
  );
  await installMockBridge(
    page,
    {
      dispatchWarmCursor: 100,
      dispatchCachedPreview: preview,
      dispatchResumeDelayMs: 5000,
      dispatch: {
        workspace: { id: workspace, name: "Loading acceptance" },
        context: {
          ...preview,
          viewerRole: "maintainer",
          issues: [{ ...issue, title: "Current issue after synchronization" }],
        },
      },
    },
    { seedPreviewFeatures: true },
  );
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-dispatch-view").click();
  await expect(
    page.getByRole("button", { name: /Saved issue ready to read/ }),
  ).toBeVisible({ timeout: 2500 });
  await expect(
    page.getByText("Showing saved issues while Dispatch checks for updates.", {
      exact: false,
    }),
  ).toBeVisible();
  await expect(page.getByTestId("dispatch-new-issue")).toBeDisabled();
  await expect(
    page.getByRole("button", { name: /Current issue after synchronization/ }),
  ).toBeVisible({ timeout: 10000 });
  await expect(page.getByTestId("dispatch-new-issue")).toBeEnabled();
  await expect(
    page.getByText("Showing saved issues while Dispatch checks for updates.", {
      exact: false,
    }),
  ).toHaveCount(0);
  const folds = await page.evaluate(
    () =>
      window.__BUZZ_E2E_COMMANDS__?.filter(
        (command) => command === "resume_tracker_context",
      ).length,
  );
  expect(folds).toBe(1);
  await page.getByRole("button", { name: "Inbox", exact: true }).click();
  await page.getByTestId("open-dispatch-view").click();
  await expect(
    page.getByRole("button", { name: /Current issue after synchronization/ }),
  ).toBeVisible({ timeout: 1000 });
});
