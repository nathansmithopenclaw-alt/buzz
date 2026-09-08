import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

const AGENT = "22".repeat(32);

test("Summary uses defaults, shared agent picker, and a guided source workflow", async ({
  page,
}) => {
  await installMockBridge(page, {
    globalAgentConfig: {
      env_vars: {},
      preferred_runtime: "codex",
      model: "gpt-5.4",
      provider: null,
    },
    managedAgents: [
      { pubkey: AGENT, name: "Summary specialist", channelNames: ["general"] },
    ],
    discoverAgentModels: {
      supportsSwitching: true,
      models: [
        { id: "gpt-5.4", name: "GPT-5.4" },
        { id: "gpt-5.3-codex", name: "GPT-5.3 Codex" },
      ],
      effortConfigId: "reasoning_effort",
      effortOptions: [
        { id: "low", name: "Low" },
        { id: "high", name: "High" },
      ],
    },
  });
  await page.goto("/pulse", { waitUntil: "domcontentloaded" });
  await page.getByTestId("pulse-tab-summary").click();
  await expect(
    page.getByRole("heading", { name: "Build your Summary" }),
  ).toBeVisible();
  await expect(page.locator("#summary-nectar-harness")).toContainText("Codex");
  await expect(page.locator("#summary-nectar-model")).toContainText("GPT-5.4");
  await expect(page.getByText("Daily time", { exact: true })).not.toBeVisible();
  await page.getByRole("button", { name: "Choose agent", exact: true }).click();
  const picker = page.getByTestId("summary-agent-picker");
  await picker
    .getByRole("textbox", { name: "Search agents" })
    .fill("specialist");
  await picker.getByText("Summary specialist", { exact: true }).click();
  await picker.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.locator("#summary-nectar-harness")).toBeVisible();
  await page.getByRole("button", { name: "Choose agent", exact: true }).click();
  await picker.getByText("Summary specialist", { exact: true }).click();
  await picker.getByRole("button", { name: "Confirm", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Edit agent setup" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await expect(page.getByText("Step 2 of 3", { exact: false })).toBeVisible();
  await expect(
    page.getByText("Direct messages", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText("Daily time", { exact: true })).not.toBeVisible();
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await expect(page.getByText("Daily time", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await expect(page.getByText("Step 2 of 3", { exact: false })).toBeVisible();
  await page.screenshot({
    path: "test-results/summary-conversations.png",
    fullPage: true,
  });
});
