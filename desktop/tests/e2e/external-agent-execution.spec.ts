import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import fs from "node:fs/promises";

const PUBKEY = "aa".repeat(32);
const THREAD = "01a08c97-4b41-7000-bbd6-b58aa4b34eef";

test.afterEach(async ({ page }, testInfo) => {
  if (testInfo.status !== testInfo.expectedStatus) {
    await fs.writeFile(
      testInfo.outputPath("routing-diagnostic.json"),
      JSON.stringify(
        await page.evaluate(() => ({
          commands: window.__BUZZ_E2E_COMMAND_LOG__,
          events: window.__BUZZ_E2E_SIGNED_EVENTS__,
          html: document.body.innerHTML.slice(0, 1000),
        })),
      ),
    );
    await testInfo.attach("routing-diagnostic", {
      body: JSON.stringify(
        await page.evaluate(() => ({
          commands: window.__BUZZ_E2E_COMMAND_LOG__,
          events: window.__BUZZ_E2E_SIGNED_EVENTS__,
        })),
      ),
      contentType: "application/json",
    });
  }
});

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => console.error("Page error:", error.message));
  page.on("console", (message) => {
    if (message.type() === "error")
      console.error("Browser error:", message.text());
  });
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: PUBKEY,
        name: "Codex Desktop",
        status: "stopped",
        channelNames: ["general"],
        externalExecution: { type: "codex_desktop", thread_id: THREAD },
      },
    ],
  });
});

test("normal picker preserves signed recipients across two sends without a local wake", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const input = page.getByTestId("message-input");
  const picker = page
    .getByTestId("message-composer")
    .getByTestId("mention-autocomplete");
  for (const text of ["First routed request", "Second routed request"]) {
    await input.fill("@Codex");
    await expect(
      picker.getByText("Codex Desktop", { exact: true }),
    ).toBeVisible();
    await input.press("Enter");
    await expect(input.locator(".mention-chip")).toHaveText("Codex Desktop");
    await page.keyboard.type(text);
    await page.getByTestId("send-message").click();
    await expect(
      page.getByTestId("message-row").filter({ hasText: text }),
    ).toBeVisible();
  }
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_COMMAND_LOG__?.filter(
            (e) => e.command === "ensure_managed_agent_for_inbound",
          ).length ?? 0,
      ),
    )
    .toBe(2);
  const result = await page.evaluate(() => ({
    commands: window.__BUZZ_E2E_COMMAND_LOG__,
    events: window.__BUZZ_E2E_SIGNED_EVENTS__,
  }));
  expect(
    result.commands?.filter((e) => e.command === "start_managed_agent"),
  ).toHaveLength(0);
  const messages =
    result.events?.filter(
      (e) => e.kind === 9 && e.content.includes("routed request"),
    ) ?? [];
  expect(messages).toHaveLength(2);
  for (const event of messages) {
    expect(event.tags.filter((t) => t[0] === "p")).toEqual([["p", PUBKEY]]);
    expect(event.tags.filter((t) => t[0] === "h")).toEqual([
      ["h", "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50"],
    ]);
  }
});

test("agent card and runtime profile open the external task without local model controls", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  const card = page.getByTestId(`managed-agent-${PUBKEY}`);
  await expect(card).toContainText("Runs in Codex Desktop");
  await expect(
    card.getByRole("button", { name: "Start Agent", exact: true }),
  ).toHaveCount(0);
  await card.click();
  await page.getByTestId("user-profile-tab-runtime").click();
  const runtime = page.getByTestId("external-execution-runtime");
  await expect(runtime).toContainText(
    "Model settings and active work are managed in Codex Desktop",
  );
  await expect(
    page.getByRole("switch", { name: "Start on launch" }),
  ).toHaveCount(0);
  await runtime.getByRole("button", { name: "Open Codex Desktop" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_COMMAND_LOG__?.filter(
            (e) => e.command === "open_external_agent_task",
          ).length ?? 0,
      ),
    )
    .toBe(1);
  expect(
    await page.evaluate(
      () =>
        window.__BUZZ_E2E_COMMAND_LOG__?.filter((e) =>
          ["start_managed_agent", "stop_managed_agent"].includes(e.command),
        ).length ?? 0,
    ),
  ).toBe(0);
});
