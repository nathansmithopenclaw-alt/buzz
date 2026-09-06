import { expect, test, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const CLAUDE_AGENT = "ca".repeat(32);
const GENERAL_CHANNEL = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

async function waitForBridge(page: Page) {
  await page.waitForFunction(
    () =>
      typeof (window as Window & { __BUZZ_E2E_SEED_ACTIVE_TURNS__?: unknown })
        .__BUZZ_E2E_SEED_ACTIVE_TURNS__ === "function",
  );
}

async function seedTurn(page: Page, kind: "turn_started" | "turn_completed") {
  await page.evaluate(
    ({ agentPubkey, channelId, eventKind }) => {
      const win = window as Window & {
        __BUZZ_E2E_SEED_ACTIVE_TURNS__?: (input: {
          agentPubkey: string;
          channelId: string;
          turnId: string;
          kind: "turn_started" | "turn_completed";
        }) => void;
      };
      win.__BUZZ_E2E_SEED_ACTIVE_TURNS__?.({
        agentPubkey,
        channelId,
        turnId: "claude-switch-turn",
        kind: eventKind,
      });
    },
    { agentPubkey: CLAUDE_AGENT, channelId: GENERAL_CHANNEL, eventKind: kind },
  );
}

async function commandCount(page: Page, command: string) {
  return page.evaluate(
    (name) =>
      (
        window as Window & { __BUZZ_E2E_COMMANDS__?: string[] }
      ).__BUZZ_E2E_COMMANDS__?.filter((entry) => entry === name).length ?? 0,
    command,
  );
}

async function openAgentEditDialog(page: Page, agentName: string) {
  await page.getByTestId("open-agents-view").click();
  await page
    .getByRole("button", { name: `${agentName} agent profile` })
    .click();
  await page.getByTestId("user-profile-edit-agent").click();
  await expect(page.getByTestId("edit-agent-dialog")).toBeVisible();
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: CLAUDE_AGENT,
        name: "Helm",
        runtime: "claude",
        status: "running",
        channelNames: ["general"],
        channelIds: [GENERAL_CHANNEL],
      },
    ],
  });
});

test("Subscription Accounts explains migration without token entry or fabricated readiness", async ({
  page,
}) => {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "agents");
  const card = page.getByTestId("settings-subscription-accounts");
  await expect(card).toContainText("No subscription accounts saved");
  await expect(
    card.getByRole("button", { name: /Sign in with openai/ }),
  ).toBeDisabled();
  await expect(
    card.getByRole("button", { name: /Sign in with xai/ }),
  ).toBeDisabled();
  const legacy = page.getByTestId("settings-claude-accounts");
  await expect(legacy).toContainText("Account A");
  await expect(legacy).toContainText("Add and name a long-lived token");
  await expect(
    card.getByRole("button", { name: /Sign in with anthropic/ }),
  ).toHaveCount(0);
  await expect(page.getByLabel("Claude OAuth token")).toHaveCount(0);
});

test("browser login remains pending until provider verification and cancellation retains recovery", async ({
  page,
}) => {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "experimental");
  await page.getByRole("switch", { name: "Subscription Accounts" }).click();
  // Navigate between Settings sections through their normal UI.
  await page.getByTestId("settings-nav-agents").click();
  const card = page.getByTestId("settings-subscription-accounts");
  await card.getByRole("button", { name: /Sign in with openai/ }).click();
  await expect(card).toContainText(
    "Waiting for browser login and verification",
  );
  await expect(card.getByRole("textbox")).toHaveCount(0);
  await card.getByRole("button", { name: "Cancel login" }).click();
  await expect(card).toContainText("Login cancelled");
  await expect(card.getByRole("button", { name: "Reconnect" })).toBeEnabled();
  expect(await commandCount(page, "begin_subscription_account_login")).toBe(1);
  expect(await commandCount(page, "cancel_subscription_account_login")).toBe(1);
});

test("legacy account selection is preserved and active turns block switching without queuing", async ({
  page,
}) => {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForBridge(page);
  await openAgentEditDialog(page, "Helm");
  const selector = page.getByTestId(
    `subscription-account-selector-${CLAUDE_AGENT}`,
  );
  const picker = selector.getByRole("button", { name: "Subscription account" });
  await expect(picker).toContainText("Legacy Claude account");
  await seedTurn(page, "turn_started");
  await expect(picker).toBeDisabled();
  await expect(selector).toContainText("Finish the active turn");
  expect(await commandCount(page, "switch_agent_subscription_account")).toBe(0);
  expect(await commandCount(page, "note_claude_account_switch_queued")).toBe(0);
  await seedTurn(page, "turn_completed");
  await expect(picker).toBeEnabled();
  await expect(picker).toContainText("Legacy Claude account");
  await picker.click();
  await page.getByRole("menuitemradio", { name: /Ambient — harness/ }).click();
  await expect(picker).toContainText("Ambient");
  expect(await commandCount(page, "switch_agent_subscription_account")).toBe(1);
});

test("unassigned agent exposes explicit ambient choice and honest running state", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: CLAUDE_AGENT,
        name: "Anvil",
        runtime: "claude",
        status: "running",
        claudeAccountId: null,
        claudeAccountSource: "credentials_file",
      },
    ],
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openAgentEditDialog(page, "Anvil");
  const selector = page.getByTestId(
    `subscription-account-selector-${CLAUDE_AGENT}`,
  );
  await expect(
    selector.getByRole("button", { name: "Subscription account" }),
  ).toContainText("Ambient");
  await expect(selector).toContainText(
    "Applies to this agent in every conversation",
  );
  await expect(selector).toContainText(
    "Waiting for runtime account confirmation",
  );
});

test("Agents profile Runtime switches accounts without opening a DM", async ({
  page,
}) => {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForBridge(page);
  await page.getByTestId("open-agents-view").click();
  await page.getByRole("button", { name: "Helm agent profile" }).click();
  const panel = page.getByTestId("user-profile-panel");
  await panel.getByRole("tab", { name: "Runtime" }).click();
  const selector = panel.getByTestId(
    `subscription-account-selector-${CLAUDE_AGENT}`,
  );
  const picker = selector.getByRole("button", { name: "Subscription account" });
  await expect(picker).toContainText("Legacy Claude account");
  await seedTurn(page, "turn_started");
  await expect(picker).toBeDisabled();
  await seedTurn(page, "turn_completed");
  await expect(picker).toBeEnabled();
  await picker.click();
  await page.getByRole("menuitemradio", { name: /Ambient — harness/ }).click();
  await expect(picker).toContainText("Ambient");
  expect(await commandCount(page, "switch_agent_subscription_account")).toBe(1);
  expect(await commandCount(page, "open_dm")).toBe(0);
  await expect(page.getByTestId("edit-agent-dialog")).toHaveCount(0);
});

test("token browser approval, nickname, Profile Runtime switch and feature-off renewal", async ({
  page,
}, testInfo) => {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "experimental");
  await page.getByRole("switch", { name: "Subscription Accounts" }).click();
  await page.getByTestId("settings-nav-agents").click();
  const card = page.getByTestId("settings-subscription-accounts");
  await card
    .getByRole("button", { name: "Add Claude long-lived token" })
    .click();
  await expect(card).toContainText("Waiting for browser approval");
  await expect(card.getByRole("textbox")).toHaveCount(0);
  const mintedAt = Math.floor(Date.now() / 1000) - 336 * 86400;
  await page.evaluate((minted) => {
    window.__BUZZ_E2E_COMPLETE_SUBSCRIPTION_TOKEN__?.(
      "mock-subscription-1",
      minted,
    );
  }, mintedAt);
  await card.getByRole("textbox", { name: /Nickname/ }).fill("Fleet token");
  await card.getByRole("button", { name: "Save nickname" }).click();
  await expect(
    card.getByRole("article", { name: "Fleet token" }),
  ).toContainText("Expires in 29 days");
  await expect(
    card.getByTestId("token-renewal-notice-mock-subscription-1"),
  ).toContainText("Choose Renew");
  await expect(card).toContainText("local OS notifications");
  await expect(card).not.toContainText("owner DM");
  await waitForAnimations(page);
  await card.getByRole("article", { name: "Fleet token" }).screenshot({
    path: testInfo.outputPath("SETTINGS_TOKEN.png"),
  });
  await page.keyboard.press("Escape");
  await page.getByTestId("open-agents-view").click();
  await page.getByRole("button", { name: "Helm agent profile" }).click();
  const panel = page.getByTestId("user-profile-panel");
  await panel.getByRole("tab", { name: "Runtime" }).click();
  const selector = panel.getByTestId(
    `subscription-account-selector-${CLAUDE_AGENT}`,
  );
  await selector.getByRole("button", { name: "Subscription account" }).click();
  const menu = page.getByRole("menu");
  await expect(
    menu.getByRole("menuitemradio", { name: /Fleet token/ }),
  ).toBeVisible();
  await menu.screenshot({ path: testInfo.outputPath("ACCOUNT_MENU.png") });
  await page.keyboard.press("End");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("alertdialog")).toContainText(
    "Stopped runtimes stay stopped",
  );
  expect(await commandCount(page, "switch_agent_subscription_account")).toBe(0);
  await page
    .getByRole("alertdialog")
    .screenshot({ path: testInfo.outputPath("ACCOUNT_CONFIRMATION.png") });
  await page
    .getByRole("button", { name: "Switch this agent", exact: true })
    .click();
  await expect(
    selector.getByRole("button", { name: "Subscription account" }),
  ).toContainText("Fleet token");
  await expect(selector.getByTestId("selected-token-expiry")).toHaveText(
    "Expires in 29 days",
  );
  // Let the profile tab transition finish before capturing visual evidence.
  await waitForAnimations(page);
  await page.screenshot({
    path: testInfo.outputPath("PROFILE_TOKEN.png"),
  });
  expect(await commandCount(page, "open_dm")).toBe(0);
  await page.keyboard.press("Escape");
  await openSettings(page, "experimental");
  await page.getByRole("switch", { name: "Subscription Accounts" }).click();
  await page.getByTestId("settings-nav-agents").click();
  await expect(
    card.getByRole("button", { name: "Add Claude long-lived token" }),
  ).toBeDisabled();
  await card.getByRole("button", { name: "Renew" }).click();
  await expect(card).toContainText("Waiting for browser approval");
  await page.evaluate((minted) => {
    window.__BUZZ_E2E_COMPLETE_SUBSCRIPTION_TOKEN__?.(
      "mock-subscription-1",
      minted + 336 * 86400,
    );
  }, mintedAt);
  await expect(
    card.getByRole("article", { name: "Fleet token" }),
  ).toContainText("Expires in 365 days");
  await expect(
    card.getByTestId("token-renewal-notice-mock-subscription-1"),
  ).toHaveCount(0);
  expect(await commandCount(page, "begin_subscription_account_token")).toBe(2);
  expect(await commandCount(page, "begin_subscription_account_login")).toBe(0);
  expect(await commandCount(page, "switch_agent_subscription_account")).toBe(1);
});
