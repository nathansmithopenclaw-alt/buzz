import { expect, test, type Page } from "@playwright/test";

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

test("Accounts settings shows live and scope-insufficient states", async ({
  page,
}) => {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "agents");

  const card = page.getByTestId("settings-claude-accounts");
  await expect(card).toContainText("Account A");
  await expect(card).toContainText("5-hour 82%");
  await expect(card).toContainText("Account B");
  await expect(card).toContainText(
    "Usage meter needs a login-minted token; re-mint with interactive login.",
  );
});

test("wrapped paste normalizes before save", async ({ page }) => {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "agents");

  const card = page.getByTestId("settings-claude-accounts");
  await card.getByLabel("Account label").fill("Wrapped token");
  await card
    .getByLabel("Claude OAuth token")
    .fill(`${"a".repeat(79)}\n  ${"b".repeat(29)}`);
  await card.getByRole("button", { name: "Verify and save" }).click();

  await expect(card).toContainText("Wrapped token");
  await expect(card).toContainText("108 chars");
});

test("switch requested mid-turn waits for completion before restart", async ({
  page,
}) => {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForBridge(page);
  await page.getByTestId("open-agents-view").click();
  await seedTurn(page, "turn_started");

  const pill = page.getByTestId(`claude-account-pill-${CLAUDE_AGENT}`);
  await pill.click();
  await page.getByRole("menuitem", { name: /Account B/ }).click();

  await expect(pill).toContainText("Switch queued");
  expect(await commandCount(page, "note_claude_account_switch_queued")).toBe(1);
  expect(await commandCount(page, "switch_agent_claude_account")).toBe(0);

  await seedTurn(page, "turn_completed");
  await expect
    .poll(() => commandCount(page, "switch_agent_claude_account"))
    .toBe(1);
  await expect(pill).toContainText("Account B");
});

test("unassigned Claude agent exposes credentials-file fallback", async ({
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
  await page.getByTestId("open-agents-view").click();

  const pill = page.getByTestId(`claude-account-pill-${CLAUDE_AGENT}`);
  await expect(pill).toContainText("Credentials file fallback");
  await expect(pill).toHaveAttribute(
    "title",
    "Falling back to credentials file (no env token)",
  );
});
