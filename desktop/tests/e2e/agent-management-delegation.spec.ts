import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";

import { installMockBridge } from "../helpers/bridge";

const FIRST = "11".repeat(32);
const SECOND = "22".repeat(32);
const GENERAL_CHANNEL = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

test("recipient target steps preserve separate field limits across Back and save together", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      { pubkey: FIRST, name: "Alpha" },
      { pubkey: SECOND, name: "Beta" },
    ],
  });
  await openAgentDefaults(page);
  await page
    .getByRole("button", { name: "Choose agents for update existing agents" })
    .click();
  const picker = page.getByTestId("agent-management-picker");
  await picker.getByTestId("agent-management-picker-all").check();
  const advance = picker.getByTestId("agent-management-picker-confirm");
  await expect(advance).toHaveText("Next");
  await advance.click();
  await picker
    .getByTestId(`agent-management-picker-agent-${SECOND}`)
    .getByRole("checkbox")
    .check();
  await advance.click();
  await picker
    .getByTestId(`agent-management-picker-agent-${FIRST}`)
    .getByRole("checkbox")
    .check();
  await picker.getByTestId("agent-management-field-displayName").uncheck();
  await expect(advance).toBeDisabled();
  await picker.getByTestId("agent-management-field-systemPrompt").check();
  await picker.getByRole("button", { name: "Back", exact: true }).click();
  await expect(
    picker.getByRole("heading", { name: "Which agents can Alpha modify?" }),
  ).toBeVisible();
  await expect(
    picker
      .getByTestId(`agent-management-picker-agent-${SECOND}`)
      .getByRole("checkbox"),
  ).toBeChecked();
  await expect(
    picker.getByTestId("agent-management-field-displayName"),
  ).toBeChecked();
  await advance.click();
  await expect(
    picker.getByTestId("agent-management-field-displayName"),
  ).not.toBeChecked();
  await expect(
    picker.getByTestId("agent-management-field-systemPrompt"),
  ).toBeChecked();
  expect(
    await invokeMock(page, "get_agent_management_delegation_set_call_count"),
  ).toBe(0);
  await advance.click();
  await expect(picker).not.toBeVisible();
  expect(
    await invokeMock(page, "get_agent_management_delegation_set_call_count"),
  ).toBe(1);
  const stored = await invokeMock<{
    agent_management_delegations: Record<
      string,
      { version: number; agentEdit: unknown }
    >;
  }>(page, "get_global_agent_config");
  expect(stored?.agent_management_delegations[FIRST]).toMatchObject({
    version: 4,
    agentEdit: {
      targetPubkeys: [SECOND],
      allowedFields: ["displayName"],
      allowSelfEdit: false,
    },
  });
  expect(stored?.agent_management_delegations[SECOND]).toMatchObject({
    version: 4,
    agentEdit: {
      targetPubkeys: [FIRST],
      allowedFields: ["systemPrompt"],
      allowSelfEdit: false,
    },
  });
});

test("canceling target selection does not grant the chosen recipient", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [{ pubkey: FIRST, name: "Alpha" }],
  });
  await openAgentDefaults(page);
  await page
    .getByRole("button", { name: "Choose agents for update existing agents" })
    .click();
  const picker = page.getByTestId("agent-management-picker");
  await picker.getByTestId("agent-management-picker-all").check();
  await picker.getByTestId("agent-management-picker-confirm").click();
  await picker
    .getByRole("button", { name: "Select all agents", exact: true })
    .click();
  await picker.getByTestId("agent-management-self-edit").check();
  await picker.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(
    await invokeMock(page, "get_agent_management_delegation_set_call_count"),
  ).toBe(0);
});

async function openAgentDefaults(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await expect(page.getByTestId("settings-view")).toBeVisible();
  await page.getByTestId("settings-nav-agents").click();
  await expect(
    page.getByTestId("agent-management-delegation-card"),
  ).toBeVisible({ timeout: 10_000 });
}

async function invokeMock<T>(
  page: import("@playwright/test").Page,
  command: string,
) {
  return await page.evaluate(async (cmd) => {
    return await (
      window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload: unknown,
        ) => Promise<T>;
      }
    ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.(cmd, null);
  }, command);
}

test("owner review completes an update request after saving instead of leaving a pending draft", async ({
  page,
}) => {
  await installMockBridge(page, {
    globalAgentConfig: {
      env_vars: { ANTHROPIC_API_KEY: "sk-ant-test-key" },
      provider: "anthropic",
      model: "claude-opus-4-5",
      preferred_runtime: "goose",
    },
    personas: [
      {
        id: "reviewed-worker",
        displayName: "Worker",
        systemPrompt: "Original instructions",
        runtime: "goose",
        provider: "anthropic",
        model: "claude-opus-4-5",
      },
    ],
    managedAgents: [
      { pubkey: FIRST, name: "Manager", channelNames: ["general"] },
    ],
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.waitForFunction(() =>
    (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).some(
      (entry) => entry.command === "agent_management_ledger",
    ),
  );
  await page.evaluate(
    ({ agentPubkey, channelId }) => {
      window.__BUZZ_E2E_SEED_LIVE_OBSERVER_EVENTS__?.({
        agentPubkey,
        events: [
          {
            seq: 1,
            timestamp: new Date().toISOString(),
            kind: "agent_management_request",
            agentIndex: 0,
            channelId,
            sessionId: null,
            turnId: null,
            payload: {
              type: "agent_management_request",
              action: "update",
              requestId: "bb72977f-b6ce-46d0-a659-8896119b0344",
              intent: "apply",
              request: {
                channelId,
                agentName: "Worker",
                displayName: "Reviewed worker",
              },
            },
          },
        ],
      });
    },
    { agentPubkey: FIRST, channelId: GENERAL_CHANNEL },
  );
  await expect(page.locator("#persona-display-name")).toHaveValue(
    "Reviewed worker",
  );
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Edit agent", exact: true }),
  ).not.toBeVisible();
  const commands = await page.evaluate(() =>
    (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).map((entry) => ({
      name: entry.command,
      action: (entry.payload as { command?: { action?: string } }).command
        ?.action,
    })),
  );
  const begin = commands.findIndex((c) => c.action === "begin_owner_review");
  const update = commands.findIndex((c) => c.name === "update_persona");
  const complete = commands.findIndex((c) => c.action === "complete");
  expect(begin).toBeGreaterThanOrEqual(0);
  expect(update).toBeGreaterThan(begin);
  expect(complete).toBeGreaterThan(update);
  expect(commands.filter((c) => c.action === "complete")).toHaveLength(1);
});

test("saving the first queued agent draft presents the second without dismissing it", async ({
  page,
}) => {
  await installMockBridge(page, {
    globalAgentConfig: {
      env_vars: { ANTHROPIC_API_KEY: "sk-ant-test-key" },
      model: "claude-opus-4-5",
      preferred_runtime: "goose",
      provider: "anthropic",
    },
    managedAgents: [
      {
        pubkey: FIRST,
        name: "Draft requester",
        channelNames: ["general"],
      },
    ],
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_LIVE_OBSERVER_EVENTS__ === "function",
  );
  await page.waitForFunction(() =>
    (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).some(
      (entry) =>
        entry.command === "agent_management_ledger" &&
        (entry.payload as { command?: { action?: string } }).command?.action ===
          "restore",
    ),
  );

  await page.evaluate(
    ({ agentPubkey, channelId }) => {
      const now = new Date().toISOString();
      window.__BUZZ_E2E_SEED_LIVE_OBSERVER_EVENTS__?.({
        agentPubkey,
        events: [
          {
            seq: 1,
            timestamp: now,
            kind: "agent_management_request",
            agentIndex: 0,
            channelId,
            sessionId: null,
            turnId: null,
            payload: {
              type: "agent_management_request",
              action: "create",
              requestId: "queued-create-1",
              intent: "draft",
              request: {
                channelId,
                displayName: "First queued draft",
                systemPrompt: "Handle the first queued task.",
                runtime: "goose",
              },
            },
          },
          {
            seq: 2,
            timestamp: now,
            kind: "agent_management_request",
            agentIndex: 0,
            channelId,
            sessionId: null,
            turnId: null,
            payload: {
              type: "agent_management_request",
              action: "create",
              requestId: "queued-create-2",
              intent: "draft",
              request: {
                channelId,
                displayName: "Second queued draft",
                systemPrompt: "Handle the second queued task.",
                runtime: "goose",
              },
            },
          },
        ],
      });
    },
    { agentPubkey: FIRST, channelId: GENERAL_CHANNEL },
  );

  const nameInput = page.locator("#persona-display-name");
  await expect(nameInput).toHaveValue("First queued draft", {
    timeout: 10_000,
  });
  await page.getByTestId("persona-dialog-submit").click();
  await expect(nameInput).toHaveValue("Second queued draft", {
    timeout: 10_000,
  });
  await page.getByTestId("persona-dialog-submit").click();
  await expect(page.getByTestId("persona-dialog")).toHaveCount(0);

  const secondRequestActions = await page.evaluate(() => {
    const commands = window.__BUZZ_E2E_COMMAND_LOG__ ?? [];
    const ledger = commands
      .filter((entry) => entry.command === "agent_management_ledger")
      .map(
        (entry) =>
          (
            entry.payload as {
              command?: { action?: string; requestId?: string };
            }
          ).command,
      )
      .filter((command) => command?.requestId === "queued-create-2")
      .map((command) => command?.action);
    const published = commands
      .filter((entry) => entry.command === "publish_agent_management")
      .map(
        (entry) =>
          (
            entry.payload as {
              command?: { action?: string; requestId?: string };
            }
          ).command,
      )
      .filter((command) => command?.requestId === "queued-create-2")
      .map((command) => command?.action);
    return { ledger, published };
  });
  expect(secondRequestActions.ledger).toEqual([
    "register",
    "begin_owner_review",
    "complete",
  ]);
  expect(
    secondRequestActions.published.filter((action) => action === "audit"),
  ).toHaveLength(1);
  expect(
    secondRequestActions.published.filter(
      (action) => action === "create_secret",
    ),
  ).toHaveLength(1);
});

test("hides pubkey and audit UUID fields", async ({ page }) => {
  await installMockBridge(page, {
    managedAgents: [
      { pubkey: FIRST, name: "Alpha" },
      { pubkey: SECOND, name: "Beta" },
    ],
  });
  await openAgentDefaults(page);
  await expect(page.locator("#agent-management-grantee-pubkey")).toHaveCount(0);
  await expect(page.locator("#agent-management-audit-channel")).toHaveCount(0);
  await expect(
    page.getByText("Grant managed colleagues narrowly scoped authority"),
  ).toBeVisible();
});

test("grant all permissions to current agents writes five explicit sets", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      { pubkey: FIRST, name: "Alpha" },
      { pubkey: SECOND, name: "Beta" },
    ],
  });
  await openAgentDefaults(page);
  await page.getByTestId("agent-management-grant-all").click();
  await page.getByTestId("agent-management-grant-all-confirm").click();
  for (const name of ["Alpha", "Beta"]) {
    await expect(
      page.getByRole("heading", { name: `Which agents can ${name} modify?` }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "Select all agents", exact: true })
      .click();
    await page.getByTestId("agent-management-picker-confirm").click();
  }
  await expect(
    page.getByTestId("agent-management-capability-agent_creation-summary"),
  ).toHaveText("2 agents");

  const count = await invokeMock<number>(
    page,
    "get_agent_management_delegation_set_call_count",
  );
  const payload = await invokeMock<
    Record<string, { scope: string; grantees?: string[] }>
  >(page, "get_agent_management_delegation_last_payload");
  expect(count).toBe(1);
  for (const name of [
    "agent_config",
    "agent_creation",
    "env_vars",
    "global_config",
    "teams",
  ]) {
    expect(payload?.[name]?.scope, name).toBe("set");
    expect(payload?.[name]?.grantees, name).toEqual([FIRST, SECOND]);
  }
});

test("adding and revoking a second grantee preserves the legacy proof", async ({
  page,
}) => {
  const legacyGrant = {
    granteePubkey: FIRST,
    auditChannelId: "7c07e659-3610-42f4-9a5e-1e9973c09da9",
    capabilities: {
      agent_config: false,
      agent_creation: true,
      env_vars: false,
      global_config: false,
      teams: false,
    },
    signedEvent: "byte-identical-legacy-proof",
  };
  await installMockBridge(page, {
    managedAgents: [
      { pubkey: FIRST, name: "Alpha" },
      { pubkey: SECOND, name: "Beta" },
    ],
    globalAgentConfig: {
      env_vars: {},
      provider: null,
      model: null,
      agent_management_delegation: legacyGrant,
      agent_management_delegations: {},
    },
  });
  await openAgentDefaults(page);
  await page
    .getByTestId("agent-management-capability-agent_creation")
    .getByRole("button")
    .click();
  await expect(page.getByTestId("agent-management-picker")).toBeVisible();
  await page
    .getByTestId(`agent-management-picker-agent-${SECOND}`)
    .getByRole("checkbox")
    .click();
  await page.getByTestId("agent-management-picker-confirm").click();
  await expect(
    page.getByTestId("agent-management-capability-agent_creation-summary"),
  ).toHaveText("2 agents");

  let stored = await invokeMock<{
    agent_management_delegation: typeof legacyGrant | null;
    agent_management_delegations: Record<
      string,
      { granteePubkey: string; signedEvent?: string }
    >;
  }>(page, "get_global_agent_config");
  expect(stored?.agent_management_delegation).toEqual(legacyGrant);
  expect(stored?.agent_management_delegations[SECOND]?.granteePubkey).toBe(
    SECOND,
  );

  await page.getByTestId(`agent-management-revoke-${SECOND}`).click();
  await expect(
    page.getByTestId("agent-management-capability-agent_creation-summary"),
  ).toHaveText("1 agent");
  stored = await invokeMock(page, "get_global_agent_config");
  expect(stored?.agent_management_delegation).toEqual(legacyGrant);
  expect(stored?.agent_management_delegations).toEqual({});
});

test("save does not overwrite a grant that landed mid-flight", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      { pubkey: FIRST, name: "Alpha" },
      { pubkey: SECOND, name: "Beta" },
    ],
    globalAgentConfig: {
      env_vars: { ANTHROPIC_API_KEY: "sk-ant-test" },
      provider: "anthropic",
      model: "claude-opus-4-5",
      preferred_runtime: "codex",
      auto_approve_agent_drafts: false,
      agent_management_delegation: {
        version: 2,
        auditChannelId: "7c07e659-3610-42f4-9a5e-1e9973c09da9",
        capabilities: {
          agent_config: { scope: "all" },
          agent_creation: { scope: "all" },
          env_vars: { scope: "all" },
          global_config: { scope: "all" },
          teams: { scope: "all" },
        },
      },
      agent_management_delegations: {},
    },
    globalConfigSaveDelayMs: 800,
  });
  await openAgentDefaults(page);
  await page
    .getByRole("switch", { name: /Automatically create agents/ })
    .click();
  const saveButton = page.getByRole("button", { name: "Save defaults" });
  await expect(saveButton).toBeEnabled({ timeout: 5_000 });
  await saveButton.click();

  await page
    .getByTestId("agent-management-capability-agent_creation")
    .getByRole("button")
    .click();
  await page
    .getByTestId(`agent-management-picker-agent-${SECOND}`)
    .getByRole("checkbox")
    .click();
  await page.getByTestId("agent-management-picker-confirm").click();
  await expect(
    page.getByTestId("agent-management-capability-agent_creation-summary"),
  ).toHaveText("1 agent");

  await expect(page.getByText("Saved.", { exact: true })).toBeVisible({
    timeout: 10_000,
  });
  await expect(
    page.getByTestId("agent-management-capability-agent_creation-summary"),
  ).toHaveText("1 agent");

  const stored = await invokeMock<{
    auto_approve_agent_drafts: boolean;
    agent_management_delegations: Record<
      string,
      { capabilities: { agent_creation: boolean } }
    >;
  }>(page, "get_global_agent_config");
  expect(stored?.auto_approve_agent_drafts).toBe(true);
  expect(
    stored?.agent_management_delegations[FIRST]?.capabilities.agent_creation,
  ).toBe(true);
  expect(
    stored?.agent_management_delegations[SECOND]?.capabilities.agent_creation,
  ).toBe(false);
});

test("permission pickers are explicit, keyboard accessible, and persist on reopen", async ({
  page,
}, testInfo) => {
  await installMockBridge(page, {
    managedAgents: [
      { pubkey: FIRST, name: "Alpha" },
      { pubkey: SECOND, name: "Beta" },
    ],
  });
  await openAgentDefaults(page);
  const card = page.getByTestId("agent-management-delegation-card");
  const choose = card.getByRole("button", {
    name: "Choose agents for update existing agents",
  });
  await expect(card.getByText("Choose agents", { exact: true })).toHaveCount(5);
  await expect(choose).toHaveAttribute("aria-haspopup", "dialog");
  await choose.focus();
  await page.keyboard.press("Enter");
  const picker = page.getByTestId("agent-management-picker");
  await expect(picker).toBeVisible();
  await picker
    .getByTestId(`agent-management-picker-agent-${FIRST}`)
    .getByRole("checkbox")
    .check();
  await picker.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(
    await invokeMock(page, "get_agent_management_delegation_set_call_count"),
  ).toBe(0);
  await choose.click();
  await expect(
    picker
      .getByTestId(`agent-management-picker-agent-${FIRST}`)
      .getByRole("checkbox"),
  ).not.toBeChecked();
  await picker
    .getByTestId(`agent-management-picker-agent-${SECOND}`)
    .getByRole("checkbox")
    .check();
  await page.getByTestId("agent-management-picker-confirm").click();
  await expect(
    picker.getByRole("heading", { name: "Which agents can Beta modify?" }),
  ).toBeVisible();
  expect(
    await invokeMock(page, "get_agent_management_delegation_set_call_count"),
  ).toBe(0);
  await expect(
    picker.getByTestId("agent-management-picker-confirm"),
  ).toBeDisabled();
  await picker
    .getByTestId(`agent-management-picker-agent-${FIRST}`)
    .getByRole("checkbox")
    .check();
  await picker.getByTestId("agent-management-field-systemPrompt").check();
  await picker.getByTestId("agent-management-picker-confirm").click();
  await expect(picker).not.toBeVisible();
  await expect(
    card.getByTestId("agent-management-capability-agent_config-summary"),
  ).toHaveText("1 agent");
  await expect(card).toBeVisible();
  await choose.click();
  await expect(
    picker
      .getByTestId(`agent-management-picker-agent-${SECOND}`)
      .getByRole("checkbox"),
  ).toBeChecked();
  await expect(
    picker
      .getByTestId(`agent-management-picker-agent-${FIRST}`)
      .getByRole("checkbox"),
  ).not.toBeChecked();
  await picker.getByRole("button", { name: "Cancel", exact: true }).click();
  await card
    .getByRole("button", { name: "Edit agent scope for Beta", exact: true })
    .click();
  await expect(
    picker
      .getByTestId(`agent-management-picker-agent-${FIRST}`)
      .getByRole("checkbox"),
  ).toBeChecked();
  await expect(
    picker
      .getByTestId(`agent-management-picker-agent-${SECOND}`)
      .getByRole("checkbox"),
  ).not.toBeChecked();
  await expect(
    picker.getByTestId("agent-management-field-systemPrompt"),
  ).toBeChecked();
  await waitForAnimations(page);
  await picker.screenshot({
    path: testInfo.outputPath("delegation-agent-picker.png"),
  });
  await picker.getByRole("button", { name: "Cancel", exact: true }).click();
  await waitForAnimations(page);
  await card.screenshot({
    path: testInfo.outputPath("delegation-choose-agents.png"),
  });
});

test("save errors remain in the picker with the draft intact for retry", async ({
  page,
}, testInfo) => {
  await installMockBridge(page, {
    managedAgents: [{ pubkey: FIRST, name: "Alpha" }],
    agentManagementDelegationErrors: [
      "Audit channel ownership is unavailable. Try again.",
    ],
    agentManagementDelegationDelayMs: 800,
  });
  await openAgentDefaults(page);
  await page
    .getByRole("switch", { name: "Create agents", exact: true })
    .click();
  const picker = page.getByTestId("agent-management-picker");
  const alpha = picker
    .getByTestId(`agent-management-picker-agent-${FIRST}`)
    .getByRole("checkbox");
  await alpha.check();
  await page.getByTestId("agent-management-picker-confirm").click();
  await expect(alpha).toBeDisabled();
  await expect(picker.getByRole("alert")).toHaveText(
    "Audit channel ownership is unavailable. Try again.",
  );
  await expect(alpha).toBeChecked();
  expect(await invokeMock(page, "get_global_agent_config")).toMatchObject({
    agent_management_delegation: null,
    agent_management_delegations: {},
  });
  await waitForAnimations(page);
  await picker.screenshot({
    path: testInfo.outputPath("delegation-save-error.png"),
  });
  await page.getByTestId("agent-management-picker-confirm").click();
  await expect(picker).not.toBeVisible();
  await expect(
    page.getByTestId("agent-management-capability-agent_creation-summary"),
  ).toHaveText("1 agent");
  expect(
    await invokeMock(page, "get_agent_management_delegation_set_call_count"),
  ).toBe(2);
  await page
    .getByRole("switch", { name: "Create agents", exact: true })
    .click();
  await expect(
    page.getByRole("switch", { name: "Create agents", exact: true }),
  ).not.toBeChecked();
});

for (const version of [1, 4]) {
  test(`adding an edit recipient preserves an existing v${version} grant`, async ({
    page,
  }) => {
    const existing = {
      granteePubkey: FIRST,
      auditChannelId: "7c07e659-3610-42f4-9a5e-1e9973c09da9",
      capabilities: {
        agent_config: true,
        agent_creation: true,
        env_vars: false,
        global_config: false,
        teams: false,
      },
      signedEvent: "unchanged-existing-proof",
      ...(version === 4
        ? {
            version: 4,
            relayScope: "wss://relay.example.test",
            agentEdit: {
              targetPubkeys: [SECOND],
              allowedFields: ["systemPrompt"],
              allowSelfEdit: false,
            },
          }
        : {}),
    };
    await installMockBridge(page, {
      managedAgents: [
        { pubkey: FIRST, name: "Alpha" },
        { pubkey: SECOND, name: "Beta" },
      ],
      globalAgentConfig: {
        env_vars: {},
        provider: null,
        model: null,
        agent_management_delegation: null,
        agent_management_delegations: { [FIRST]: existing },
      },
    });
    await openAgentDefaults(page);
    await page
      .getByRole("button", { name: "Choose agents for update existing agents" })
      .click();
    const picker = page.getByTestId("agent-management-picker");
    await picker
      .getByTestId(`agent-management-picker-agent-${SECOND}`)
      .getByRole("checkbox")
      .check();
    await picker.getByTestId("agent-management-picker-confirm").click();
    await expect(
      picker.getByRole("heading", { name: "Which agents can Beta modify?" }),
    ).toBeVisible();
    await picker
      .getByTestId(`agent-management-picker-agent-${FIRST}`)
      .getByRole("checkbox")
      .check();
    await picker.getByTestId("agent-management-picker-confirm").click();
    await expect(picker).not.toBeVisible();
    const stored = await invokeMock<{
      agent_management_delegations: Record<string, unknown>;
    }>(page, "get_global_agent_config");
    expect(stored?.agent_management_delegations[FIRST]).toEqual(existing);
    await page
      .getByRole("button", { name: "Edit agent scope for Beta", exact: true })
      .click();
    await expect(
      picker.getByRole("heading", { name: "Which agents can Beta modify?" }),
    ).toBeVisible();
    await picker.getByTestId("agent-management-field-systemPrompt").check();
    await picker.getByTestId("agent-management-picker-confirm").click();
    await expect(picker).not.toBeVisible();
    expect(
      (
        await invokeMock<{
          agent_management_delegations: Record<string, unknown>;
        }>(page, "get_global_agent_config")
      )?.agent_management_delegations[FIRST],
    ).toEqual(existing);
    await page.getByTestId(`agent-management-revoke-${SECOND}`).click();
    await expect(
      page.getByTestId(`agent-management-revoke-${SECOND}`),
    ).not.toBeVisible();
    expect(
      (
        await invokeMock<{
          agent_management_delegations: Record<string, unknown>;
        }>(page, "get_global_agent_config")
      )?.agent_management_delegations[FIRST],
    ).toEqual(existing);
  });
}
