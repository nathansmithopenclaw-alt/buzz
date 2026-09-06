import { expect, test, type Page } from "@playwright/test";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const AGENT = TEST_IDENTITIES.charlie.pubkey;
const CHANNEL = "94a444a4-c0a3-5966-ab05-530c6ddc2301";
const SESSION = "codex-existing-conversation";
const config = (value: string) => ({
  liveEffortSwitching: true,
  configOptions: [
    {
      id: "reasoning_effort",
      name: "Reasoning effort",
      type: "select",
      category: "thought_level",
      currentValue: value,
      options: [
        { value: "low", name: "Low" },
        { value: "high", name: "High" },
      ],
    },
  ],
});

async function seed(
  page: Page,
  kind: string,
  payload: unknown,
  seq: number,
  sessionId = SESSION,
) {
  await page.evaluate(
    ({ agentPubkey, channelId, sessionId, kind, payload, seq }) => {
      window.__BUZZ_E2E_SEED_LIVE_OBSERVER_EVENTS__?.({
        agentPubkey,
        events: [
          {
            seq,
            timestamp: new Date().toISOString(),
            kind,
            payload,
            agentIndex: 0,
            channelId,
            sessionId,
            turnId: null,
          },
        ],
      });
    },
    { agentPubkey: AGENT, channelId: CHANNEL, sessionId, kind, payload, seq },
  );
}

async function openActivity(page: Page, status: string) {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: AGENT,
        name: "Codex Colleague",
        status: "running",
        channelNames: ["agents"],
      },
    ],
    observerControlResults: [{ type: "switch_effort", status }],
  });
  await page.goto(`/#/channels/${CHANNEL}?agentSession=${AGENT}`);
  await expect(page.getByTestId("agent-session-thread-panel")).toBeVisible();
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_LIVE_OBSERVER_EVENTS__ === "function",
  );
  await seed(page, "session_config_captured", config("low"), 1);
  const picker = page.getByTestId("conversation-effort-picker");
  await expect(picker).toBeVisible();
  return picker;
}

test("live effort waits for the exact adapter receipt and keeps the conversation", async ({
  page,
}) => {
  const picker = await openActivity(page, "queued");
  await picker.getByLabel("Thinking level").click();
  await page.getByRole("menuitemradio", { name: "High", exact: true }).click();
  await expect(picker.getByRole("status")).toContainText("Queued");
  await expect(picker.getByLabel("Thinking level")).toContainText("Low");
  await expect(picker.getByLabel("Thinking level")).toBeDisabled();
  const request = await page.evaluate(() =>
    window.__BUZZ_E2E_OBSERVER_CONTROLS__?.at(-1),
  );
  expect(request).toMatchObject({
    agentPubkey: AGENT,
    payload: {
      type: "switch_effort",
      channelId: CHANNEL,
      sessionId: SESSION,
      effort: "high",
      requestId: expect.any(String),
    },
  });
  if (!request) throw new Error("Expected a recorded effort request");
  const payload = request.payload as Record<string, unknown>;
  await seed(
    page,
    "control_result",
    { ...payload, status: "applied", requestId: "old-replayed-pick" },
    2,
  );
  await expect(picker.getByRole("status")).toContainText("Queued");
  await seed(page, "session_config_captured", config("high"), 3);
  await seed(page, "control_result", { ...payload, status: "applied" }, 4);
  await expect(picker.getByRole("status")).toHaveText(
    "Thinking level applied to this conversation.",
  );
  await expect(picker.getByLabel("Thinking level")).toContainText("High");
  await expect(picker.getByLabel("Thinking level")).toBeEnabled();
  await picker.screenshot({
    path: "test-results/codex-thinking-level/live-applied.png",
  });
});

test("adapter rejection leaves the reported level unchanged", async ({
  page,
}) => {
  const picker = await openActivity(page, "rejected");
  await picker.getByLabel("Thinking level").click();
  await page.getByRole("menuitemradio", { name: "High", exact: true }).click();
  await expect(picker.getByRole("status")).toContainText("rejected");
  await expect(picker.getByLabel("Thinking level")).toContainText("Low");
  await expect(picker.getByLabel("Thinking level")).toBeEnabled();
});

test("multiple reported sessions require a conversation choice", async ({
  page,
}) => {
  const picker = await openActivity(page, "queued");
  await seed(
    page,
    "session_config_captured",
    config("low"),
    2,
    "sibling-conversation",
  );
  await expect(
    picker.getByLabel("Conversation", { exact: true }),
  ).toContainText("Choose a conversation");
  await expect(picker.getByLabel("Thinking level")).toBeDisabled();
  expect(
    await page.evaluate(() => window.__BUZZ_E2E_OBSERVER_CONTROLS__?.length),
  ).toBe(0);
});

async function openProfile(
  page: Page,
  overrides: Parameters<typeof installMockBridge>[1] = {},
) {
  await installMockBridge(page, {
    acpRuntimesCatalog: [
      {
        id: "codex",
        label: "Codex",
        avatar_url: "",
        availability: "available",
        command: "codex-acp",
        binary_path: "/test/codex-acp",
        default_args: [],
        mcp_command: null,
        install_hint: "",
        install_instructions_url: "",
        can_auto_install: false,
        underlying_cli_path: "/test/codex",
      },
    ],
    discoverAgentModels: {
      models: [{ id: "gpt-6-astra", name: "GPT-6 Astra" }],
      supportsSwitching: true,
      effortConfigId: "reasoning_effort",
      effortOptions: [
        { id: "low", name: "Low" },
        { id: "high", name: "High" },
      ],
    },
    personas: [
      {
        id: "codex-effort-persona",
        displayName: "Codex Colleague",
        systemPrompt: "",
        runtime: "codex",
        model: "gpt-6-astra[low]",
        provider: null,
      },
    ],
    managedAgents: [
      {
        pubkey: AGENT,
        name: "Codex Colleague",
        personaId: "codex-effort-persona",
        status: "stopped",
        model: "gpt-6-astra[low]",
        effortLevel: "low",
        runtime: "codex",
        channelNames: ["agents"],
      },
    ],
    ...overrides,
  });
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await page
    .getByRole("button", { name: "Codex Colleague agent profile", exact: true })
    .click();
  await page.getByTestId("user-profile-edit-agent").click();
  const dialog = page.getByTestId("persona-dialog");
  return dialog;
}

test("profile edit saves canonical effort only on Save and keeps a clean model label", async ({
  page,
}) => {
  const dialog = await openProfile(page);
  await expect(dialog.locator("#edit-agent-effort")).toContainText("Low");
  await expect(dialog.locator("#persona-model")).toContainText("GPT-6 Astra");
  await expect(dialog.locator("#persona-model")).not.toContainText("[low]");
  await dialog.locator("#edit-agent-effort").click();
  await page.getByRole("menuitemradio", { name: "High", exact: true }).click();
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.getByTestId("user-profile-edit-agent").click();
  await expect(dialog.locator("#edit-agent-effort")).toContainText("Low");
  await dialog.locator("#edit-agent-effort").click();
  await page.getByRole("menuitemradio", { name: "High", exact: true }).click();
  await dialog.screenshot({
    path: "test-results/codex-thinking-level/saved-default.png",
  });
  await page.getByTestId("persona-dialog-submit").click();
  await expect(dialog).not.toBeVisible();
  const saved = await page.evaluate(
    () =>
      window.__BUZZ_E2E_COMMAND_LOG__
        ?.filter((entry) => entry.command === "update_managed_agent")
        .at(-1)?.payload,
  );
  expect(saved).toMatchObject({
    input: { pubkey: AGENT, effortLevel: "high" },
  });
  await page.getByTestId("user-profile-edit-agent").click();
  await expect(dialog.locator("#edit-agent-effort")).toContainText("High");
});

test("failed save preserves the saved effort and keeps the editor open", async ({
  page,
}) => {
  const dialog = await openProfile(page, {
    updateManagedAgentError: "Verification save rejected",
  });
  await dialog.locator("#edit-agent-effort").click();
  await page.getByRole("menuitemradio", { name: "High", exact: true }).click();
  await page.getByTestId("persona-dialog-submit").click();
  await expect(dialog).toBeVisible();
  await expect(
    page.getByText("Verification save rejected", { exact: false }),
  ).toBeVisible();
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.getByTestId("user-profile-edit-agent").click();
  await expect(dialog.locator("#edit-agent-effort")).toContainText("Low");
});

test("failed model discovery preserves a visible saved effort", async ({
  page,
}) => {
  const dialog = await openProfile(page, {
    discoverAgentModelsError: "Verification discovery unavailable",
  });
  await expect(
    dialog.getByText("Saved thinking level: low.", { exact: false }),
  ).toBeVisible();
  await expect(page.getByTestId("persona-dialog-submit")).toBeEnabled();
  await page.getByTestId("persona-dialog-submit").click();
  await expect(dialog).not.toBeVisible();
  const update = await page.evaluate(
    () =>
      window.__BUZZ_E2E_COMMAND_LOG__
        ?.filter((e) => e.command === "update_managed_agent")
        .at(-1)?.payload,
  );
  expect(update).toBeUndefined();
  await page.getByTestId("user-profile-edit-agent").click();
  await expect(
    dialog.getByText("Saved thinking level: low.", { exact: false }),
  ).toBeVisible();
});
