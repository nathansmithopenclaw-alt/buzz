import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const FIRST = "11".repeat(32);
const SECOND = "22".repeat(32);

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
      agent_config: true,
      agent_creation: false,
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
    .getByTestId("agent-management-capability-agent_config")
    .getByRole("button")
    .click();
  await expect(page.getByTestId("agent-management-picker")).toBeVisible();
  await page
    .getByTestId(`agent-management-picker-agent-${SECOND}`)
    .getByRole("checkbox")
    .click();
  await page.getByTestId("agent-management-picker-confirm").click();
  await expect(
    page.getByTestId("agent-management-capability-agent_config-summary"),
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
    page.getByTestId("agent-management-capability-agent_config-summary"),
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
