import { expect, test, type Page } from "@playwright/test";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";
const WORKSPACE = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const ID = "f".repeat(64);
async function open(page: Page, fail = false) {
  await installMockBridge(page, {
    ...(fail ? { sendMessageErrors: ["Capture send failed"] } : {}),
    dispatch: {
      workspace: { id: WORKSPACE, name: "Capture workspace" },
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
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: "general",
          }) ?? false,
      ),
    )
    .toBe(true);
}
async function emit(
  page: Page,
  id = ID,
  pubkey = TEST_IDENTITIES.alice.pubkey,
  content = "A message worth tracking",
) {
  await page.evaluate(
    ({ id, pubkey, content }) =>
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        id,
        pubkey,
        content,
      }),
    { id, pubkey, content },
  );
  const row = page.locator(`[data-message-id="${id}"]`).first();
  await expect(row).toBeVisible();
  return row;
}
test("hover capture exposes the first-row action, tooltip and linked issue", async ({
  page,
}, info) => {
  await open(page);
  const row = await emit(page);
  await row.hover();
  const track = row.getByRole("button", {
    name: "Track in Dispatch",
    exact: true,
  });
  await expect(track).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: info.outputPath("hover-row.png") });
  await track.hover();
  await expect(page.getByRole("tooltip")).toHaveText("Track in Dispatch (T)");
  await waitForAnimations(page);
  await page.screenshot({ path: info.outputPath("track-tooltip.png") });
  await track.click();
  const dialog = page.getByTestId("conversation-dispatch-dialog");
  await dialog.getByLabel("Outcome or next step").fill("Follow through");
  await dialog.getByRole("button", { name: "Create linked issue" }).click();
  await expect(
    dialog.getByRole("button", { name: "Open issue" }),
  ).toBeVisible();
  const roots = await page.evaluate(() =>
    (window.__BUZZ_E2E_PUBLISHED_DISPATCH_EVENTS__ ?? [])
      .filter((e) => e.kind === 40020)
      .map((e) => JSON.parse(e.content)),
  );
  expect(roots).toHaveLength(1);
  expect(roots[0].source_event_id).toBe(ID);
});
test("T on a focused message opens capture without taking text input", async ({
  page,
}) => {
  await open(page);
  const row = await emit(page);
  await row.focus();
  await page.keyboard.press("t");
  const dialog = page.getByTestId("conversation-dispatch-dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByLabel("Outcome or next step").fill("t is ordinary text");
  await expect(dialog.getByLabel("Outcome or next step")).toHaveValue(
    "t is ordinary text",
  );
  await page.keyboard.press("Escape");
  const composer = page.locator('[contenteditable="true"]').first();
  await composer.fill("t");
  await expect(dialog).toHaveCount(0);
  await expect(composer).toContainText("t");
});
test("Cmd+Shift+T captures the newest own message in the open channel", async ({
  page,
}) => {
  await open(page);
  await emit(page, "a".repeat(64), "deadbeef".repeat(8), "My latest message");
  await emit(
    page,
    ID,
    TEST_IDENTITIES.alice.pubkey,
    "Someone else's later message",
  );
  await page.keyboard.press("Meta+Shift+t");
  const dialog = page.getByTestId("conversation-dispatch-dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByLabel("Outcome or next step").fill("My follow-up");
  await dialog.getByRole("button", { name: "Create linked issue" }).click();
  await expect(
    dialog.getByRole("button", { name: "Open issue" }),
  ).toBeVisible();
  const roots = await page.evaluate(() =>
    (window.__BUZZ_E2E_PUBLISHED_DISPATCH_EVENTS__ ?? [])
      .filter((e) => e.kind === 40020)
      .map((e) => JSON.parse(e.content)),
  );
  expect(roots[0].source_event_id).toBe("a".repeat(64));
});
test("leading /track sends then prefills capture from the accepted event", async ({
  page,
}) => {
  await open(page);
  const composer = page.locator('[contenteditable="true"]').first();
  await composer.fill("/track Follow up tomorrow\nMore context");
  await page.keyboard.press("Enter");
  const dialog = page.getByTestId("conversation-dispatch-dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByLabel("Outcome or next step")).toHaveValue(
    "Follow up tomorrow",
  );
  const row = page
    .getByTestId("message-row")
    .filter({ hasText: "Follow up tomorrow" })
    .first();
  await expect(row).not.toContainText("/track");
  const source = await row.getAttribute("data-message-id");
  await dialog.getByRole("button", { name: "Create linked issue" }).click();
  await expect(
    dialog.getByRole("button", { name: "Open issue" }),
  ).toBeVisible();
  const roots = await page.evaluate(() =>
    (window.__BUZZ_E2E_PUBLISHED_DISPATCH_EVENTS__ ?? [])
      .filter((e) => e.kind === 40020)
      .map((e) => JSON.parse(e.content)),
  );
  expect(roots[0].source_event_id).toBe(source);
});
test("failed /track send never opens capture or creates an issue", async ({
  page,
}) => {
  await open(page, true);
  const composer = page.locator('[contenteditable="true"]').first();
  await composer.fill("/track This must remain unsent");
  await page.keyboard.press("Enter");
  await expect(
    page.getByText("Capture send failed", { exact: false }).first(),
  ).toBeVisible();
  await expect(page.getByTestId("conversation-dispatch-dialog")).toHaveCount(0);
  expect(
    await page.evaluate(() =>
      (window.__BUZZ_E2E_PUBLISHED_DISPATCH_EVENTS__ ?? []).filter(
        (e) => e.kind === 40020,
      ),
    ),
  ).toHaveLength(0);
});
test("pin toggle persists the channel workflow and disables without deleting", async ({
  page,
}) => {
  await open(page);
  await page.getByTestId("channel-management-trigger").click();
  await page.getByTestId("channel-management-edit").click();
  const toggle = page.getByRole("switch", {
    name: "Pin reactions create Dispatch issues",
  });
  await expect(toggle).not.toBeChecked();
  await toggle.click();
  await expect(toggle).toBeChecked();
  const read = () =>
    page.evaluate(
      async () =>
        await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.(
          "get_channel_workflows",
          { channelId: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50" },
        ),
    ) as Promise<Array<{ id: string; definition: Record<string, unknown> }>>;
  const workflows = await read();
  const workflow = workflows.find(
    (w) => w.definition.name === "dispatch-capture",
  );
  expect(workflow?.definition).toEqual({
    name: "dispatch-capture",
    enabled: true,
    trigger: {
      on: "reaction_added",
      emoji: "📌",
      filter: `trigger_channel_id == "${WORKSPACE}"`,
    },
    steps: [{ id: "capture", action: "dispatch.create_issue" }],
  });
  await toggle.click();
  await expect(toggle).not.toBeChecked();
  const disabled = (await read()).find((w) => w.id === workflow?.id);
  expect(disabled?.definition.enabled).toBe(false);
  expect(
    await page.evaluate(() =>
      window.__BUZZ_E2E_COMMANDS__?.includes("delete_workflow"),
    ),
  ).toBe(false);
});
