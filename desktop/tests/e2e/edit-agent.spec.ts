import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const BAKED_DEFAULTS = [
  { key: "BUZZ_AGENT_PROVIDER", value: "anthropic", masked: false },
  {
    key: "BUZZ_AGENT_MODEL",
    value: "claude-opus-4-8",
    masked: false,
  },
  { key: "BUZZ_AGENT_THINKING_EFFORT", value: "high", masked: false },
  { key: "ANTHROPIC_API_KEY", value: "sk-ant-baked-test", masked: true },
];

// Edit-agent dialog coverage (Phase 1B.3b-pre). Written against TODAY'S
// EditAgentDialog, before the B3b re-host, so the re-host is guarded by a
// pre-existing spec rather than one written alongside it.
//
// Mock-boundary caveat: the e2eBridge `update_managed_agent` handler echoes
// name/model/provider/systemPrompt/envVars/respondTo/respondToAllowlist into
// the mock store — it does NOT
// model the diff-based partial-update wire semantics (change-detected-or-omit,
// tri-state provider, harnessOverride derivation), and it ignores
// agentCommand/harnessOverride entirely. This spec therefore pins UI behavior
// (open → edit → save → persisted in UI), not wire semantics. The inherit
// toggle is not reachable here at all (see the routing pin below) — its
// behavior is covered by B3b's component-level pinning test (inherit-toggle
// → gate → submit); wire semantics stay component-test territory
// (personaRuntimeModel.test.mjs).

// Tyler's pubkey maps to gooseSurface in the mock bridge (runtimeId "goose"),
// which supports LLM provider selection — same seed the readiness-screenshot
// spec uses for its edit-dialog shot.
const AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const AGENT_NAME = "Tyler Agent";
const PERSONA_ID = "persona-edit-e2e";

/**
 * Open the Edit Agent dialog for the seeded managed agent via the profile
 * panel (agents view → agent card → Edit quick action) — EditAgentDialog's
 * only mount path.
 */
async function openEditDialog(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();

  const agentButton = page.getByRole("button", {
    name: `${AGENT_NAME} agent profile`,
  });
  await expect(agentButton).toBeVisible({ timeout: 10_000 });
  await agentButton.click();

  await expect(page.getByTestId("user-profile-panel")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByTestId("user-profile-edit-agent").click();

  await expect(page.getByTestId("edit-agent-dialog")).toBeVisible({
    timeout: 10_000,
  });
  // Provider field visible = runtime catalog loaded and form settled.
  await expect(page.locator("#edit-agent-llm-provider")).toBeVisible({
    timeout: 10_000,
  });
}

/**
 * Pick an option from a PersonaDropdownField (menu-based, not a native
 * <select> — Create's fields are selects, Edit's are not).
 */
async function pickDropdownOption(
  page: import("@playwright/test").Page,
  triggerId: string,
  optionName: string | RegExp,
) {
  await page.locator(`#${triggerId}`).click();
  await page.getByRole("menuitemradio", { name: optionName }).click();
}

test.describe("agent definition dialog", () => {
  test("owner-only-access build shows disabled agent access with an explanation", async ({
    page,
  }) => {
    await installMockBridge(page, {
      ownerOnlyAccessBuild: true,
      bakedBuildEnv: BAKED_DEFAULTS,
    });
    await page.goto("/");
    await page.getByTestId("open-agents-view").click();
    await page.getByTestId("new-agent-card").click();

    const dialog = page.getByRole("dialog");
    await dialog.getByRole("button", { name: "Advanced", exact: true }).click();

    await expect(dialog.getByTestId("agent-respond-to")).toBeVisible();
    await expect(dialog.locator("#agent-respond-to")).toBeDisabled();
    await expect(dialog.locator("#agent-respond-to")).toContainText(
      "Only me (default)",
    );
    await expect(
      dialog.getByTestId("agent-respond-to-disabled-reason"),
    ).toHaveText("This build disallows changing this setting.");
  });
});

test.describe("edit agent dialog", () => {
  test("owner-only-access build shows a disabled owner-only access control with an explanation", async ({
    page,
  }) => {
    await installMockBridge(page, {
      ownerOnlyAccessBuild: true,
      bakedBuildEnv: BAKED_DEFAULTS,
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: ["agents"],
          respondTo: "anyone",
        },
      ],
    });

    await openEditDialog(page);

    const accessControl = page.getByTestId("agent-respond-to");
    await expect(accessControl).toBeVisible();
    await expect(page.locator("#agent-respond-to")).toBeDisabled();
    await expect(page.locator("#agent-respond-to")).toContainText(
      "Only me (default)",
    );
    await expect(
      page.getByTestId("agent-respond-to-disabled-reason"),
    ).toHaveText("This build disallows changing this setting.");
  });

  test("OSS build keeps the managed-agent access control", async ({ page }) => {
    await installMockBridge(page, {
      bakedBuildEnv: BAKED_DEFAULTS,
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
    });

    await openEditDialog(page);

    await expect(page.getByTestId("agent-respond-to")).toBeVisible();
  });

  test("edits the agent name and persists it across a dialog reopen", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
    });

    await openEditDialog(page);

    const nameInput = page.locator("#edit-agent-name");
    await expect(nameInput).toHaveValue(AGENT_NAME);
    await nameInput.fill("Tyler Agent Renamed");

    await page.getByTestId("edit-agent-dialog-submit").click();
    await expect(page.getByTestId("edit-agent-dialog")).not.toBeVisible();

    // Reopen: the dialog re-reads the managed-agents store, proving the save
    // survived the dialog lifecycle rather than living in local state. (The
    // panel HEADER is not asserted — it renders the relay profile name, which
    // the update path does not touch.)
    await page.getByTestId("user-profile-edit-agent").click();
    await expect(page.locator("#edit-agent-name")).toHaveValue(
      "Tyler Agent Renamed",
      { timeout: 10_000 },
    );
  });

  test("changes the model via custom entry and persists it", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
    });

    await openEditDialog(page);

    // Pick a provider so model discovery has a scope, then set a custom model.
    await pickDropdownOption(page, "edit-agent-llm-provider", "Anthropic");
    await pickDropdownOption(page, "edit-agent-model", "Custom model...");
    await page.locator("#edit-agent-custom-model").fill("claude-opus-4-5");
    // Anthropic requires a credential before save unlocks.
    await page.getByLabel("Anthropic API Key").fill("sk-test-edit-agent-e2e");

    const submit = page.getByTestId("edit-agent-dialog-submit");
    await expect(submit).toBeEnabled({ timeout: 10_000 });
    await submit.click();
    await expect(page.getByTestId("edit-agent-dialog")).not.toBeVisible();

    await page.getByTestId("user-profile-edit-agent").click();
    await expect(page.getByTestId("edit-agent-dialog")).toBeVisible({
      timeout: 10_000,
    });
    // The value entered through Custom is now recognized by live discovery,
    // so reopen renders the persisted value in the dropdown rather than
    // mounting the custom input. Provider must round-trip independently too.
    await expect(page.locator("#edit-agent-model")).toContainText(
      "claude-opus-4-5",
    );
    await expect(page.locator("#edit-agent-llm-provider")).toContainText(
      "Anthropic",
    );
  });

  test("keeps the custom command visible without opening Advanced", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
    });

    await openEditDialog(page);

    const advanced = page.getByRole("button", {
      name: "Advanced",
      exact: true,
    });
    await expect(advanced).toHaveAttribute("aria-expanded", "false");
    await pickDropdownOption(page, "edit-agent-runtime", "Custom command");
    await expect(page.locator("#edit-agent-command")).toBeVisible();
    await expect(advanced).toHaveAttribute("aria-expanded", "false");
  });

  test("marks a missing advanced credential without opening Advanced", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
    });

    await openEditDialog(page);

    const advanced = page.getByRole("button", {
      name: "Advanced",
      exact: true,
    });
    await expect(advanced).toHaveAttribute("aria-expanded", "false");
    await pickDropdownOption(page, "edit-agent-llm-provider", "Databricks v2");
    await expect(advanced).toHaveAttribute("aria-expanded", "false");
    await expect(
      page.getByTestId("edit-agent-advanced-required-badge"),
    ).toHaveText("Required");
    await expect(page.getByTestId("edit-agent-dialog-submit")).toBeDisabled();

    await advanced.click();
    await expect(page.getByLabel("Value for DATABRICKS_HOST")).toBeVisible();
  });

  test("shows baked defaults in the instance editor", async ({ page }) => {
    await installMockBridge(page, {
      bakedBuildEnv: BAKED_DEFAULTS,
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
    });

    await openEditDialog(page);

    await expect(page.locator("#edit-agent-llm-provider")).toHaveText(
      "Anthropic (inherited from build)",
    );
    await expect(page.locator("#edit-agent-model")).toHaveText(
      "Inherit build default (claude-opus-4-8)",
    );
    const defaults = page.getByTestId("agent-ai-defaults-notice");
    await expect(
      defaults.getByText("Anthropic", { exact: true }),
    ).toBeVisible();
    await expect(
      defaults.getByText("claude-opus-4-8", { exact: true }),
    ).toBeVisible();
  });

  test("explicit global defaults override baked labels in the instance editor", async ({
    page,
  }) => {
    await installMockBridge(page, {
      bakedBuildEnv: BAKED_DEFAULTS,
      globalAgentConfig: {
        provider: "anthropic",
        model: "claude-opus-4-5",
        env_vars: { BUZZ_AGENT_THINKING_EFFORT: "low" },
      },
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
    });

    await openEditDialog(page);

    await expect(page.locator("#edit-agent-llm-provider")).toHaveText(
      "Use agent defaults (anthropic)",
    );
    await expect(page.locator("#edit-agent-model")).toHaveText(
      "Use agent defaults (claude-opus-4-5)",
    );
    const defaults = page.getByTestId("agent-ai-defaults-notice");
    await expect(
      defaults.getByText("Anthropic", { exact: true }),
    ).toBeVisible();
    await expect(
      defaults.getByText("claude-opus-4-5", { exact: true }),
    ).toBeVisible();
  });

  test("profile Edit routes persona-linked agents to the definition editor", async ({
    page,
  }) => {
    // Routing pin for handleEditAgent (UserProfilePanel): when the agent has
    // a resolvable non-built-in persona, the Edit quick action opens the
    // DEFINITION editor (persona dialog), not EditAgentDialog. The instance
    // editor (and its inherit-runtime toggle) is reachable for persona-linked
    // agents only via the requestOpenEditAgent event (ConfigNudgeCard) — no
    // plain UI path — so its inherit-toggle behavior is covered by B3b's
    // component-level pinning test, not e2e.
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          personaId: PERSONA_ID,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
      personas: [
        {
          id: PERSONA_ID,
          displayName: "Edit E2E Persona",
          systemPrompt: "You are the edit-agent e2e persona.",
        },
      ],
    });

    await page.goto("/");
    await page.getByTestId("open-agents-view").click();

    // Persona-linked agents render grouped under the persona's card name.
    const agentButton = page.getByRole("button", {
      name: "Edit E2E Persona agent profile",
    });
    await expect(agentButton).toBeVisible({ timeout: 10_000 });
    await agentButton.click();

    await expect(page.getByTestId("user-profile-panel")).toBeVisible({
      timeout: 10_000,
    });
    await page.getByTestId("user-profile-edit-agent").click();

    // Definition editor opens; the instance editor does not.
    await expect(page.getByTestId("persona-dialog")).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.getByTestId("edit-agent-dialog")).not.toBeVisible();
    // And it is the persona's record that's being edited.
    await expect(page.locator("#persona-display-name")).toHaveValue(
      "Edit E2E Persona",
    );
  });

  test("Claude Edit Agent shows the complete catalog with native effort directly below model", async ({
    page,
  }) => {
    const advertisedClaudeCatalog = [
      ["opus", "Claude Opus (current)"],
      ["sonnet", "Claude Sonnet (current)"],
      ["haiku", "Claude Haiku (current)"],
      ["claude-fable-5", "Claude Fable 5"],
      ["claude-opus-5", "Claude Opus 5"],
      ["claude-opus-4-8", "Claude Opus 4.8"],
      ["claude-opus-4-7", "Claude Opus 4.7"],
      ["claude-opus-4-6", "Claude Opus 4.6"],
      ["claude-opus-4-5-20251101", "Claude Opus 4.5 (2025-11-01)"],
      ["claude-opus-4-1-20250805", "Claude Opus 4.1 (2025-08-05)"],
      ["claude-opus-4-20250514", "Claude Opus 4 (2025-05-14)"],
      ["claude-sonnet-5", "Claude Sonnet 5"],
      ["claude-sonnet-4-6", "Claude Sonnet 4.6"],
      ["claude-sonnet-4-5-20250929", "Claude Sonnet 4.5 (2025-09-29)"],
      ["claude-sonnet-4-20250514", "Claude Sonnet 4 (2025-05-14)"],
      ["claude-haiku-4-5-20251001", "Claude Haiku 4.5 (2025-10-01)"],
    ] as const;
    const canonicalClaudeCatalog = advertisedClaudeCatalog.slice(3);
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        {
          id: "claude",
          label: "Claude Code",
          avatar_url: "",
          availability: "available",
          command: "claude-agent-acp",
          binary_path: "/usr/local/bin/claude-agent-acp",
          default_args: [],
          mcp_command: null,
          install_hint: "",
          install_instructions_url: null,
          can_auto_install: false,
          requires_external_cli: true,
          underlying_cli_path: "/usr/local/bin/claude",
          source: "builtin",
        },
      ],
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          runtime: "claude",
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
      discoverAgentModels: {
        models: advertisedClaudeCatalog.map(([id, name]) => ({
          id,
          name,
        })),
        supportsSwitching: true,
        effortConfigId: "effort",
        effortOptions: ["default", "low", "medium", "high", "xhigh", "max"].map(
          (id) => ({ id, name: id === "default" ? "Default" : id }),
        ),
        fastModeConfigId: "fast",
        fastModeEnabled: false,
        fastModeDescription: "Fast mode",
      },
    });

    await page.goto("/");
    await page.getByTestId("open-agents-view").click();
    await page
      .getByRole("button", { name: `${AGENT_NAME} agent profile` })
      .click();
    await page.getByTestId("user-profile-edit-agent").click();

    const dialog = page.getByTestId("edit-agent-dialog");
    const model = page.locator("#edit-agent-model");
    const effort = page.getByTestId("instance-agent-effort-select");
    const fastMode = page.getByTestId("agent-fast-mode-field");
    await expect(dialog).toBeVisible({ timeout: 10_000 });
    await expect(model).toBeVisible({ timeout: 10_000 });
    await expect(effort).toBeVisible({ timeout: 10_000 });
    await expect(fastMode).toBeVisible({ timeout: 10_000 });
    await expect(fastMode).toContainText(
      "Uses usage credits at a higher rate and has separate rate limits.",
    );
    const [modelBox, effortBox, fastModeBox] = await Promise.all([
      model.boundingBox(),
      effort.boundingBox(),
      fastMode.boundingBox(),
    ]);
    expect(effortBox?.y ?? 0).toBeGreaterThan(
      (modelBox?.y ?? 0) + (modelBox?.height ?? 0),
    );
    expect(fastModeBox?.y ?? 0).toBeGreaterThan(
      (effortBox?.y ?? 0) + (effortBox?.height ?? 0),
    );
    await dialog.screenshot({
      path: "test-results/model-selector-effort/01-claude-model-and-effort.png",
    });
    await fastMode.scrollIntoViewIfNeeded();
    await dialog.screenshot({
      path: "test-results/model-selector-effort/02-claude-fast-mode.png",
    });

    await dialog.evaluate((root) => {
      for (const element of [root, ...root.querySelectorAll("*")]) {
        if (element.scrollHeight > element.clientHeight) element.scrollTop = 0;
      }
    });
    await model.click();
    for (const [, name] of canonicalClaudeCatalog) {
      await expect(
        page
          .getByRole("menuitemradio")
          .filter({ has: page.getByText(name, { exact: true }) }),
      ).toBeAttached();
    }
    for (const [, aliasName] of advertisedClaudeCatalog.slice(0, 3)) {
      await expect(
        page
          .getByRole("menuitemradio")
          .filter({ has: page.getByText(aliasName, { exact: true }) }),
      ).toHaveCount(0);
    }
    await page.getByRole("menu").screenshot({
      path: "test-results/model-selector-effort/03-claude-full-catalog.png",
    });

    await page
      .getByRole("menuitemradio")
      .filter({ hasText: "Claude Opus 4.1 (2025-08-05)" })
      .click();
    await expect(model).toHaveText("Claude Opus 4.1 (2025-08-05)");
    await page.getByTestId("edit-agent-dialog-submit").click();
    await expect(dialog).not.toBeVisible();

    await page.getByTestId("user-profile-edit-agent").click();
    await expect(model).toHaveText("Claude Opus 4.1 (2025-08-05)", {
      timeout: 10_000,
    });
  });
});

// The real dialog and IPC mapper are exercised; persistence/failure is mocked
// at the native boundary. Rust tests cover the locked record update separately.
test("effort selection is singular, Cancel-safe, and retryable after Save failure", async ({
  page,
}) => {
  await installMockBridge(page, {
    bakedBuildEnv: BAKED_DEFAULTS,
    managedAgents: [
      {
        pubkey: AGENT_PUBKEY,
        name: AGENT_NAME,
        status: "stopped",
        channelNames: ["agents"],
      },
    ],
  });
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function",
  );
  await page.evaluate(() => {
    const win = window as unknown as {
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          args?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
      __effortTest: { value: string | null; writes: number; fail: boolean };
    };
    const original = win.__TAURI_INTERNALS__.invoke;
    const state = { value: "low" as string | null, writes: 0, fail: true };
    win.__effortTest = state;
    win.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "persist_agent_effort_level")
        throw new Error("Direct effort writes are forbidden");
      if (command === "update_managed_agent") {
        state.writes++;
        if (state.fail) throw new Error("Effort save failed for test");
        const result = (await original(command, args)) as {
          agent: Record<string, unknown>;
        };
        const input = args?.input as { effortLevel?: string | null };
        if (input.effortLevel !== undefined) state.value = input.effortLevel;
        result.agent.effort_level = state.value;
        return result;
      }
      const result = await original(command, args);
      if (command === "list_managed_agents") {
        const response = result as Array<Record<string, unknown>>;
        for (const agent of response) agent.effort_level = state.value;
      }
      if (command === "get_agent_config_surface") {
        return {
          ...(result as Record<string, unknown>),
          effortConfigId: "depth",
          effortOptions: [
            { value: "low", displayName: "Low" },
            { value: "high", displayName: "High" },
          ],
        };
      }
      return result;
    };
  });
  // Do not navigate/reload after installing the boundary wrapper.
  await page.getByTestId("open-agents-view").click();
  await page
    .getByRole("button", { name: `${AGENT_NAME} agent profile` })
    .click();
  await page.getByTestId("user-profile-edit-agent").click();
  const effort = page.locator("#edit-agent-effort");
  await expect(effort).toContainText("Low");
  await expect(page.getByTestId("instance-agent-effort-select")).toHaveCount(0);
  await pickDropdownOption(page, "edit-agent-effort", "High");
  const state = () =>
    page.evaluate(
      () =>
        (
          window as unknown as {
            __effortTest: {
              value: string | null;
              writes: number;
              fail: boolean;
            };
          }
        ).__effortTest,
    );
  expect((await state()).writes).toBe(0);
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  expect((await state()).value).toBe("low");
  await page.getByTestId("user-profile-edit-agent").click();
  await expect(effort).toContainText("Low");
  await pickDropdownOption(page, "edit-agent-effort", "High");
  await page.getByTestId("edit-agent-dialog-submit").click();
  await expect(
    page.getByText("Effort save failed for test", { exact: true }),
  ).toBeVisible();
  await expect(effort).toContainText("High");
  expect((await state()).value).toBe("low");
  await page.evaluate(() => {
    (
      window as unknown as { __effortTest: { fail: boolean } }
    ).__effortTest.fail = false;
  });
  await page.getByTestId("edit-agent-dialog-submit").click();
  await expect(page.getByTestId("edit-agent-dialog")).not.toBeVisible();
  expect((await state()).value).toBe("high");
  await page.getByTestId("user-profile-edit-agent").click();
  await expect(effort).toContainText("High");
});

for (const channelName of ["general", "alice-tyler"]) {
  test(`hover card edits agent without leaving ${channelName}`, async ({
    page,
  }) => {
    await installMockBridge(page, {
      bakedBuildEnv: BAKED_DEFAULTS,
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: [channelName],
        },
      ],
    });
    const channelId =
      channelName === "general"
        ? "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50"
        : "f48efb06-0c93-5025-aac9-2e646bb6bfa8";
    await page.goto(`/#/channels/${channelId}`);
    await expect
      .poll(() =>
        page.evaluate(
          (name) =>
            (
              window as Window & {
                __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                  channelName: string;
                }) => boolean;
              }
            ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: name,
            }) ?? false,
          channelName,
        ),
      )
      .toBe(true);
    await page.evaluate(
      ({ channelName, pubkey }) =>
        (
          window as Window & {
            __BUZZ_E2E_EMIT_MOCK_MESSAGE__: (input: {
              channelName: string;
              pubkey: string;
              content: string;
            }) => unknown;
          }
        ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__({
          channelName,
          pubkey,
          content: "Hover editor test message",
        }),
      { channelName, pubkey: AGENT_PUBKEY },
    );
    const route = page.url();
    // Every message author avatar uses the same shared hover card.
    const trigger = page
      .getByTestId("message-row")
      .filter({ hasText: "Hover editor test message" })
      .locator("button")
      .first();
    await trigger.hover();
    const action = page.getByTestId(`user-profile-edit-agent-${AGENT_PUBKEY}`);
    await expect(action).toBeVisible();
    await action.click();
    const dialog = page.getByTestId("edit-agent-dialog");
    await expect(dialog).toBeVisible();
    await page.mouse.move(1, 1);
    await expect(dialog).toBeVisible();
    expect(page.url()).toBe(route);
    await expect(page.getByTestId("user-profile-panel")).toHaveCount(0);
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(dialog).toHaveCount(0);
    expect(page.url()).toBe(route);
    await trigger.hover();
    await action.click();
    await dialog
      .getByRole("textbox", { name: "Agent name", exact: true })
      .fill("Edited from conversation");
    await waitForAnimations(page);
    await page.screenshot({
      path: `test-results/hover-edit-${channelName}.png`,
    });
    await dialog
      .getByRole("button", { name: "Save changes", exact: true })
      .click();
    await expect(dialog).toHaveCount(0);
    expect(page.url()).toBe(route);
    await trigger.hover();
    await action.click();
    await expect(
      dialog.getByRole("textbox", { name: "Agent name", exact: true }),
    ).toHaveValue("Edited from conversation");
  });
}
