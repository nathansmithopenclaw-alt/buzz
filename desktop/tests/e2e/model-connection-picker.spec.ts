import { expect, test, type Page } from "@playwright/test";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const MODEL = "Qwen/model";
const SOURCE = "saved-source-one";
const CONNECTION_KEY = "BUZZ_MODEL_CONNECTION_ID";
const agentName = "Connection Picker Agent";
const runtimes = [
  {
    id: "goose",
    command: "goose",
    label: "Goose",
    protocol: "chat_completions",
    modelKey: "GOOSE_MODEL",
    providerKey: "GOOSE_PROVIDER",
  },
  {
    id: "codex",
    command: "codex-acp",
    label: "Codex",
    protocol: "responses",
    modelKey: null,
    providerKey: null,
  },
  {
    id: "buzz-agent",
    command: "buzz-agent",
    label: "Buzz Agent",
    protocol: "chat_completions",
    modelKey: "BUZZ_AGENT_MODEL",
    providerKey: "BUZZ_AGENT_PROVIDER",
  },
];

type Probe = { writes: Array<Record<string, unknown>>; inference: number };
type BridgeWindow = Window & {
  __connectionPicker: Probe;
  __TAURI_INTERNALS__: {
    invoke: (
      command: string,
      args?: Record<string, unknown>,
    ) => Promise<unknown>;
  };
};

async function fixture(
  page: Page,
  runtime: (typeof runtimes)[number],
  missing = false,
  reason: string | null = null,
) {
  await installMockBridge(page, {
    acpRuntimesCatalog: [
      {
        id: runtime.id,
        label: runtime.label,
        avatar_url: "",
        availability: "available",
        command: runtime.command,
        binary_path: `/local/bin/${runtime.command}`,
        default_args: [],
        model_env_var: runtime.modelKey,
        provider_env_var: runtime.providerKey,
        model_connection: {
          protocol: runtime.protocol,
          setupReason: null,
          defaultEffort: null,
        },
        source: "builtin",
        install_hint: "",
        can_auto_install: false,
      },
    ],
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.tyler.pubkey,
        name: agentName,
        runtime: runtime.id,
        status: "stopped",
        channelNames: ["agents"],
        provider: "openai",
        model: missing || reason ? MODEL : "hosted-model",
        envVars: {
          OPENAI_API_KEY: "fixture-key",
          ...(missing || reason ? { [CONNECTION_KEY]: SOURCE } : {}),
        },
      },
    ],
    discoverAgentModels: {
      models: [{ id: "hosted-model", name: "Hosted model" }],
      supportsSwitching: true,
    },
  });
  await page.goto("/");
  await page.waitForFunction(() =>
    Boolean(
      (window as unknown as Partial<BridgeWindow>).__TAURI_INTERNALS__?.invoke,
    ),
  );
  await page.evaluate(
    ({ missing, protocol, reason }) => {
      const win = window as unknown as BridgeWindow;
      const original = win.__TAURI_INTERNALS__.invoke;
      win.__connectionPicker = { writes: [], inference: 0 };
      win.__TAURI_INTERNALS__.invoke = async (command, args) => {
        if (command === "codex_connection_workspace_reason") return reason;
        if (command === "list_model_connections")
          return missing
            ? []
            : [
                {
                  id: "saved-source-one",
                  name: "opus",
                  base_url: "http://127.0.0.1:8010/v1",
                  models: ["Qwen/model"],
                  has_key: false,
                  tested_model: "Qwen/model",
                  tested_at: 1,
                  qualification: {
                    revision: 1,
                    model: "Qwen/model",
                    context_window: 65536,
                    output_token_limit: 8192,
                    protocols: [
                      {
                        protocol,
                        tool_round_trip: true,
                        reason: null,
                        effort_levels: [],
                      },
                    ],
                  },
                },
              ];
        if (
          command === "test_model_connection" ||
          command === "discover_model_servers"
        ) {
          win.__connectionPicker.inference++;
          throw new Error("Picker must never qualify or scan a server");
        }
        if (command === "update_managed_agent")
          win.__connectionPicker.writes.push(
            args?.input as Record<string, unknown>,
          );
        return original(command, args);
      };
    },
    { missing, protocol: runtime.protocol, reason },
  );
  await page.getByTestId("open-agents-view").click();
  await page
    .getByRole("button", { name: `${agentName} agent profile` })
    .click();
  await page.getByTestId("user-profile-edit-agent").click();
  await expect(page.getByTestId("edit-agent-dialog")).toBeVisible();
}

for (const runtime of runtimes) {
  test(`${runtime.label} model picker saves the exact source and raw model without inference`, async ({
    page,
  }) => {
    await fixture(page, runtime);
    const dialog = page.getByTestId("edit-agent-dialog");
    await page.locator("#edit-agent-model").click();
    await expect(page.getByText("opus", { exact: true })).toBeVisible();
    // Custom IDs belong to the hosted catalog, before any saved source header.
    expect(
      await page.getByText("opus", { exact: true }).evaluate((header) => {
        const custom = Array.from(
          header.parentElement?.querySelectorAll('[role="menuitemradio"]') ??
            [],
        ).find((row) => row.textContent?.includes("Custom model..."));
        return (
          !!custom &&
          !!(
            custom.compareDocumentPosition(header) &
            Node.DOCUMENT_POSITION_FOLLOWING
          )
        );
      }),
    ).toBe(true);
    const choice = page.getByRole("menuitemradio", {
      name: MODEL,
      exact: true,
    });
    await choice.scrollIntoViewIfNeeded();
    await page.screenshot({
      path: `test-results/connection-picker-${runtime.id}.png`,
    });
    await choice.click();
    await expect(page.locator("#edit-agent-model")).toContainText(MODEL);
    if (runtime.id === "codex") {
      const notice = dialog.getByTestId("connection-runtime-policy");
      await expect(notice).toContainText("65,536");
      await expect(notice).toContainText("49,152");
      await expect(notice).toContainText("Skips user and project MCP servers");
      await expect(notice).not.toContainText("8,192");
    }

    await dialog
      .getByRole("button", { name: "Save changes", exact: true })
      .click();
    await expect(dialog).toHaveCount(0);
    const result = await page.evaluate(
      () => (window as unknown as BridgeWindow).__connectionPicker,
    );
    expect(result.inference).toBe(0);
    expect(result.writes).toHaveLength(1);
    expect(result.writes[0].model).toBe(MODEL);
    expect(result.writes[0].envVars).toMatchObject({
      [CONNECTION_KEY]: SOURCE,
      OPENAI_API_KEY: "fixture-key",
    });
    expect(result.writes[0]).not.toHaveProperty("subscriptionAccountId");
  });
}

test("missing saved source is named and blocks Save without a hosted fallback", async ({
  page,
}) => {
  await fixture(page, runtimes[0], true);
  const dialog = page.getByTestId("edit-agent-dialog");
  await expect(dialog.getByRole("alert")).toContainText(SOURCE);
  await expect(
    dialog.getByRole("button", { name: "Save changes", exact: true }),
  ).toBeDisabled();
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(
    (
      await page.evaluate(
        () => (window as unknown as BridgeWindow).__connectionPicker,
      )
    ).writes,
  ).toEqual([]);
});

test("Codex project MCP refusal names the file and servers and blocks Save", async ({
  page,
}) => {
  const reason =
    "Saved-connection Codex cannot start: /workspace/.codex/config.toml declares MCP servers (accounts, deployments). Use a workspace without project MCP servers.";
  await fixture(page, runtimes[1], false, reason);
  const dialog = page.getByTestId("edit-agent-dialog");
  await expect(dialog.getByRole("alert")).toContainText(reason);
  await expect(
    dialog.getByRole("button", { name: "Save changes", exact: true }),
  ).toBeDisabled();
  const result = await page.evaluate(
    () => (window as unknown as BridgeWindow).__connectionPicker,
  );
  expect(result.inference).toBe(0);
  expect(result.writes).toHaveLength(0);
});
