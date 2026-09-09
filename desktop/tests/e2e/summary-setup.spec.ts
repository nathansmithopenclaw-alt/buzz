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
    acpRuntimesCatalog: ["buzz-agent", "codex"].map((id) => ({
      id,
      label: id === "codex" ? "Codex" : "Buzz Agent",
      availability: "available" as const,
      command: id,
      binary_path: `/local/bin/${id}`,
      default_args: [],
      model_env_var: "CODEX_MODEL",
      source: "preset" as const,
      install_hint: "",
      can_auto_install: false,
    })),
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
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-pulse-view").click();
  await page.getByTestId("pulse-tab-summary").click();
  await page.getByRole("button", { name: "New Summary", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Build your Summary" }),
  ).toBeVisible();
  await expect(page.locator("#summary-nectar-harness")).toContainText("Codex");
  await expect(page.locator("#summary-nectar-model")).toContainText("GPT-5.4");
  await expect(
    page.getByText("Run on a schedule", { exact: true }),
  ).not.toBeVisible();
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
  await expect(page.getByText("Step 2 of 4", { exact: false })).toBeVisible();
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Direct messages", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Run on a schedule", { exact: true }),
  ).not.toBeVisible();
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await expect(
    page.getByText("Run on a schedule", { exact: true }),
  ).toBeVisible();
  await page.locator('label[for="wf-trigger-frequency-weekly"]').click();
  await page.locator('label[for="wf-trigger-weekday-5"]').click();
  await page.locator("#wf-trigger-time").fill("17:30");
  await page.getByRole("button", { name: "Save Summary", exact: true }).click();
  await page.getByRole("button", { name: "Open", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Run summary", exact: true }),
  ).toBeVisible();
  const saved = await page.evaluate(() =>
    JSON.parse(sessionStorage.getItem("buzz-e2e-summary-events") ?? "[]"),
  );
  expect(JSON.parse(saved.at(-1).content).schedule).toEqual({
    cron: "30 17 * * 1,5",
  });
  await page.getByRole("button", { name: "Run summary", exact: true }).click();
  await expect(
    page.getByRole("status").filter({ hasText: "Summary requested" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Edit Summary", exact: true }).click();
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await expect(page.locator("#wf-trigger-time")).toHaveValue("17:30");
  await expect(page.getByLabel("Friday", { exact: true })).toBeChecked();
  await page
    .getByRole("switch", { name: "Run on a schedule", exact: true })
    .click();
  await page.getByRole("button", { name: "Save Summary", exact: true }).click();

  await expect(
    page.getByRole("button", { name: "Run summary", exact: true }),
  ).toBeEnabled();
  const manual = await page.evaluate(() =>
    JSON.parse(sessionStorage.getItem("buzz-e2e-summary-events") ?? "[]"),
  );
  expect(JSON.parse(manual.at(-1).content).schedule).toBeNull();
  await page.getByRole("button", { name: "Run summary", exact: true }).click();
  await expect(
    page.getByRole("status").filter({ hasText: "Summary requested" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Edit Summary", exact: true }).click();
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await expect(page.getByText("Step 2 of 4", { exact: false })).toBeVisible();
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await page.screenshot({
    path: "test-results/summary-conversations.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await page
    .getByRole("button", { name: "Back to Summaries", exact: true })
    .click();
  await page.getByRole("button", { name: "Duplicate", exact: true }).click();
  await page
    .getByRole("textbox", { name: "Summary name" })
    .fill("Afternoon support");
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await page
    .getByRole("radio", { name: "Selected people", exact: true })
    .check();
  await page
    .getByRole("button", { name: "Choose people (0)", exact: true })
    .click();
  const people = page.getByTestId("summary-people-picker");
  await expect(
    people.getByRole("textbox", { name: "Search people" }),
  ).toBeVisible();
  await expect(
    people.getByRole("checkbox", { name: "Summary specialist" }),
  ).toHaveCount(0);
  await expect(people.getByRole("checkbox")).toHaveCount(3);
  await people.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.getByRole("radio", { name: "Only me", exact: true }).check();
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await page
    .getByRole("switch", { name: "Run on a schedule", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Set daily times", exact: true })
    .click();
  await page.getByLabel("Daily time 1", { exact: true }).fill("09:15");
  await page.getByRole("button", { name: "Add time", exact: true }).click();
  await page.getByLabel("Daily time 2", { exact: true }).fill("17:45");
  await page.getByRole("button", { name: "Save Summary", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Open", exact: true }),
  ).toHaveCount(2);
  const definitions = await page.evaluate(() =>
    JSON.parse(sessionStorage.getItem("buzz-e2e-summary-events") ?? "[]").map(
      (e: { content: string }) => JSON.parse(e.content),
    ),
  );
  const ids = new Set(
    definitions.map((d: { summary_id: string }) => d.summary_id),
  );
  expect(ids.size).toBe(2);
  expect(definitions.at(-1).schedule.dailyTimes).toEqual(["09:15", "17:45"]);
  await page.screenshot({
    path: "test-results/summary-collection.png",
    fullPage: true,
  });
});
