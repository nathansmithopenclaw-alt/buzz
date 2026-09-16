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
  await page
    .getByRole("textbox", { name: "Summary name" })
    .fill("Daily overview");
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
    page.getByRole("status").filter({ hasText: "Summary request delivered" }),
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
    page.getByRole("status").filter({ hasText: "already has an active run" }),
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
  await page
    .getByRole("button", { name: "Actions for Daily overview", exact: true })
    .click();
  await page.getByRole("menuitem", { name: "Duplicate", exact: true }).click();
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
  await page
    .getByRole("textbox", { name: "Search Summaries" })
    .fill("Afternoon");
  await expect(
    page.getByRole("button", { name: "Open", exact: true }),
  ).toHaveCount(1);
  await page.getByRole("textbox", { name: "Search Summaries" }).fill("");
  await expect(
    page.getByRole("button", { name: "Open", exact: true }),
  ).toHaveCount(2);
  await page
    .getByRole("button", { name: "Actions for Afternoon support" })
    .click();
  await page.getByRole("menuitem", { name: "Pause", exact: true }).click();
  await page.getByRole("button", { name: "Paused", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Open", exact: true }),
  ).toHaveCount(1);
  await page.getByRole("button", { name: "All", exact: true }).click();
  await page.evaluate(() => {
    const key = "buzz-e2e-summary-events";
    const events = JSON.parse(sessionStorage.getItem(key) ?? "[]");
    const definition = events.find(
      (e: { content: string }) =>
        JSON.parse(e.content).name === "Daily overview",
    );
    const config = JSON.parse(definition.content);
    for (const [index, title] of [
      "Morning report",
      "Evening report",
    ].entries()) {
      events.push({
        id: String(index + 7).repeat(64),
        pubkey: config.reporter_pubkey,
        kind: 40030,
        created_at: 1788960000 + index * 3600,
        sig: "0".repeat(128),
        tags: [
          ["p", definition.pubkey],
          ["owner", definition.pubkey],
          ["summary", config.summary_id],
          ["t", "buzz-brief"],
        ],
        content: JSON.stringify({ v: 2, title, markdown: `${title} body` }),
      });
    }
    sessionStorage.setItem(key, JSON.stringify(events));
  });
  await page.reload();
  await page.getByTestId("open-pulse-view").click();
  await page.getByTestId("pulse-tab-summary").click();
  await expect(
    page.getByRole("article", { name: "Daily overview", exact: true }),
  ).toContainText("2 reports");
  await page.screenshot({
    path: "test-results/summary-collection.png",
    fullPage: true,
  });
  await page
    .getByRole("article", { name: "Daily overview", exact: true })
    .getByRole("button", { name: "Open", exact: true })
    .click();
  await expect(
    page.getByText("Evening report body", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Morning report body", { exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Back to reports", exact: true })
    .click();
  await page
    .getByRole("button", { name: /^Morning report.*Daily overview/ })
    .click();
  await expect(
    page.getByText("Morning report body", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Evening report body", { exact: true }),
  ).toHaveCount(0);
  await page.screenshot({
    path: "test-results/summary-report-history.png",
    fullPage: true,
  });
  await page.setViewportSize({ width: 600, height: 800 });
  await expect(
    page.getByRole("article", { name: "Selected report" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Edit Summary", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Edit agent setup", exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: "test-results/summary-agent-controls.png",
    fullPage: true,
  });
  await page
    .getByRole("button", { name: "Edit agent setup", exact: true })
    .click();
  await expect(page.getByRole("dialog").last()).toBeVisible();
  await page
    .getByRole("dialog")
    .last()
    .getByRole("button", { name: "Cancel", exact: true })
    .click();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await page
    .getByRole("button", { name: "Manage reports", exact: true })
    .click();
  await page.setViewportSize({ width: 1280, height: 900 });
  await expect(
    page.getByRole("button", { name: /^Evening report.*Summary agent/ }),
  ).toBeVisible();
  await page.screenshot({
    path: "test-results/summary-report-library.png",
    fullPage: true,
  });
  await page
    .getByLabel("Search reports", { exact: true })
    .fill("Morning report");
  await expect(
    page.getByRole("button", { name: /^Evening report.*Summary agent/ }),
  ).toHaveCount(0);
  await page
    .getByRole("checkbox", { name: "Select this page", exact: true })
    .check();
  await page
    .getByRole("button", { name: "Archive selected", exact: true })
    .click();
  await expect(
    page.getByText("No matching reports on this page."),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Archived reports", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: /^Morning report.*Summary agent/ }),
  ).toBeVisible();
  await page.screenshot({
    path: "test-results/summary-archive.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: /^Restore Morning report/ }).click();
  await expect(
    page.getByText("No matching reports on this page."),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Current reports", exact: true })
    .click();
  await page
    .getByRole("checkbox", { name: "Select this page", exact: true })
    .check();
  await page
    .getByRole("button", { name: "Delete selected", exact: true })
    .click();
  await expect(page.getByRole("alertdialog")).toContainText("every recipient");
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(
    page.getByRole("button", { name: /^Morning report.*Summary agent/ }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Delete selected", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Delete reports", exact: true })
    .click();
  await expect(
    page.getByText("No matching reports on this page."),
  ).toBeVisible();
});

async function seedHistory(page: import("@playwright/test").Page) {
  page.on("pageerror", (error) =>
    console.error("Summary browser error:", error.message),
  );
  await installMockBridge(page, {
    managedAgents: [
      { pubkey: AGENT, name: "Summary specialist", channelNames: ["general"] },
    ],
  });
  await page.addInitScript(
    ({ agent }) => {
      const owner = "deadbeef".repeat(8),
        at = 1788960000;
      const id = (i: number) =>
        `00000000-0000-4000-8000-${String(i).padStart(12, "0")}`;
      const eventId = (i: number) => i.toString(16).padStart(64, "0");
      const events = [];
      for (let i = 0; i < 205; i++) {
        const config = {
          v: 2,
          summary_id: id(i),
          name:
            i === 0
              ? "Busy summary"
              : i === 1
                ? "Quiet summary"
                : `Archived setup ${i}`,
          reporter_pubkey: agent,
          audience: { mode: "only_me" },
          read_delegation: true,
          channels: [],
          dms: [],
          exclude: [],
          auto_include: false,
          schedule: null,
          timezone: "UTC",
          paused: false,
          archived: i > 1,
        };
        events.push({
          id: eventId(1000 + i),
          pubkey: owner,
          kind: 30623,
          created_at: at + i,
          tags: [
            ["d", `summary:${config.summary_id}`],
            ["p", owner],
            ["p", agent],
            ["t", "buzz-brief"],
          ],
          content: JSON.stringify(config),
          sig: "0".repeat(128),
        });
      }
      for (let i = 0; i < 56; i++) {
        const summary = id(i === 0 ? 1 : 0);
        events.push({
          id: eventId(2000 + i),
          pubkey: agent,
          kind: 40030,
          created_at: at + i,
          tags: [
            ["p", owner],
            ["owner", owner],
            ["summary", summary],
            ["t", "buzz-brief"],
          ],
          content: JSON.stringify({
            v: 2,
            title: i === 0 ? "Older quiet report" : `Repeated report ${i}`,
            markdown:
              i === 0
                ? "Older report remains discoverable."
                : `Report body ${i}`,
            since: at - 3600,
            until: at,
          }),
          sig: "0".repeat(128),
        });
      }
      sessionStorage.setItem("buzz-e2e-summary-events", JSON.stringify(events));
    },
    { agent: AGENT },
  );
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-pulse-view").click();
  await page.getByTestId("pulse-tab-summary").click();
}

test("older Summary history is independent of the first global page and links open the report", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 900, height: 700 });
  await seedHistory(page);
  const quiet = page.getByRole("article", {
    name: "Quiet summary",
    exact: true,
  });
  await expect(quiet).toContainText("1 report");
  await quiet.getByRole("button", { name: "Open", exact: true }).click();
  await expect(
    page.getByText("Older report remains discoverable.", { exact: true }),
  ).toBeInViewport();
  await expect(
    page.getByRole("heading", { name: "Older quiet report", exact: true }),
  ).toBeFocused();
  await expect(
    page.getByRole("article", { name: "Selected report" }),
  ).toBeInViewport();
  await page.screenshot({ path: testInfo.outputPath("summary-reader.png") });
  await page
    .getByRole("button", { name: "Regenerate this period", exact: true })
    .click();
  await expect(page.getByRole("alertdialog")).toContainText(
    "Scheduled reporting progress will not change",
  );
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  const report = (2000).toString(16).padStart(64, "0");
  await page.evaluate((id) => {
    window.location.hash = `/pulse?summaryReport=${id}`;
  }, report);
  await expect(
    page.getByRole("article", { name: "Selected report" }),
  ).toContainText("Older report remains discoverable.");
});

test("archived setups beyond the old limit stay archived after editing", async ({
  page,
}) => {
  await seedHistory(page);
  await page
    .getByRole("button", { name: "Summary archive", exact: true })
    .click();
  await page
    .getByText("Archived Summary setups (203)", { exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Search archived Summary setups" })
    .fill("Archived setup 204");
  await page.getByRole("button", { name: "Open", exact: true }).click();
  await page.getByRole("button", { name: "Edit Summary", exact: true }).click();
  await page
    .getByRole("textbox", { name: "Summary name" })
    .fill("Renamed archived setup");
  for (let i = 0; i < 3; i++)
    await page.getByRole("button", { name: "Continue", exact: true }).click();
  await page.getByRole("button", { name: "Save Summary", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Renamed archived setup", exact: true }),
  ).toBeVisible();
  const archived = await page.evaluate(
    () =>
      JSON.parse(
        JSON.parse(
          sessionStorage.getItem("buzz-e2e-summary-events") ?? "[]",
        ).at(-1).content,
      ).archived,
  );
  expect(archived).toBe(true);
});
