import { openSettings } from "../helpers/settings";
import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

test("discover once, save a connection, and select its model in an agent", async ({
  page,
}) => {
  await installMockBridge(page, {
    globalAgentConfig: {
      preferred_runtime: "deepseek",
      provider: "buzz-local",
      model: null,
      env_vars: {},
    },
    acpRuntimesCatalog: [
      {
        id: "deepseek",
        label: "DeepSeek Harness",
        availability: "available",
        command: "dsh",
        binary_path: "/local/bin/dsh",
        default_args: ["--profile", "buzz-acp"],
        model_env_var: "DSH_MODEL",
        provider_env_var: "DSH_PROVIDER",
        provider_options: [["buzz-local", "Saved model connection"]],
        source: "preset",
        install_hint: "",
        can_auto_install: false,
      },
      {
        id: "prime",
        label: "Prime Agent",
        availability: "available",
        command: "prime-agent",
        binary_path: "/local/bin/prime-agent",
        default_args: ["--mode", "acp"],
        model_env_var: "BUZZ_PRIME_MODEL",
        provider_env_var: "BUZZ_PRIME_PROVIDER",
        provider_options: [["buzz-local", "Saved model connection"]],
        source: "preset",
        install_hint: "",
        can_auto_install: false,
      },
    ],
    discoverAgentModels: {
      models: [{ id: "Qwen/model", name: "Qwen/model" }],
      supportsSwitching: true,
    },
  });
  await page.goto("/");
  await page.waitForFunction(() =>
    Boolean(
      (window as unknown as { __TAURI_INTERNALS__?: { invoke?: unknown } })
        .__TAURI_INTERNALS__?.invoke,
    ),
  );
  await page.evaluate(() => {
    const target = window as unknown as {
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
      __MODEL_CONNECTION_WRITES__: unknown[];
    };
    const original = target.__TAURI_INTERNALS__.invoke;
    let rows: unknown[] = [];
    target.__MODEL_CONNECTION_WRITES__ = [];
    target.__TAURI_INTERNALS__.invoke = async (command, payload) => {
      if (command === "list_model_connections") return rows;
      if (command === "check_model_connection") return ["Qwen/model"];
      if (command === "discover_model_servers")
        return {
          servers: [
            {
              name: "Spark fixture",
              base_url: "http://192.168.1.50:8010/v1",
              models: ["Qwen/model"],
              needs_key: false,
            },
          ],
          notice: "Fixture local discovery",
        };
      if (command === "save_model_connection") {
        target.__MODEL_CONNECTION_WRITES__.push(payload);
        const row = {
          id: "connection-fixture",
          name: "Spark fixture",
          base_url: "http://192.168.1.50:8010/v1",
          models: ["Qwen/model"],
          has_key: false,
          tested_model: null,
          tested_at: null,
        };
        rows = [row];
        return row;
      }
      return original(command, payload);
    };
  });
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  // The shared Create dialog title is selected independently of nested connection dialogs.
  const dialog = page.getByRole("dialog").first();
  await dialog.getByRole("tab", { name: "Customize for this agent" }).click();
  await dialog
    .getByRole("button", { name: "Find model servers", exact: true })
    .click();
  const connection = page.getByRole("dialog", {
    name: "Connect a model server",
    exact: true,
  });
  await expect(
    connection.getByText("Spark fixture", { exact: true }),
  ).toBeVisible();
  await connection
    .getByRole("button", { name: "Connect", exact: true })
    .click();
  await expect(connection).toHaveCount(0);
  await expect(
    dialog.getByRole("button", { name: "Model connection", exact: true }),
  ).toContainText("Spark fixture");
  const model = dialog.getByLabel(/^Model\s*\*?$/);
  await waitForAnimations(page);
  await model.click();
  await waitForAnimations(page);
  await page.getByRole("button", { name: /Qwen\/model/ }).click();
  await expect(model).toContainText("Qwen");
  await waitForAnimations(page);
  await dialog.getByLabel(/^Agent harness/).click();
  await waitForAnimations(page);
  await page
    .getByRole("menuitemradio", { name: "Prime Agent", exact: true })
    .click();
  await expect(
    dialog.getByRole("button", { name: "Model connection", exact: true }),
  ).toContainText("Spark fixture");
  await expect(model).toContainText("Qwen");

  await dialog
    .getByLabel("Agent name", { exact: true })
    .fill("Connection fixture");
  await dialog
    .getByLabel("Agent instructions", { exact: true })
    .fill("Fixture tasks only.");
  await dialog.getByRole("button", { name: "Add agent", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const target = window as unknown as {
          __BUZZ_E2E_COMMAND_LOG__: Array<{
            command: string;
            payload?: {
              input?: {
                provider?: string;
                model?: string;
                envVars?: Record<string, string>;
              };
            };
          }>;
        };
        const input = target.__BUZZ_E2E_COMMAND_LOG__.find(
          (row) => row.command === "create_persona",
        )?.payload?.input;
        return (
          input && {
            provider: input.provider,
            model: input.model,
            connection: input.envVars?.BUZZ_MODEL_CONNECTION_ID,
          }
        );
      }),
    )
    .toEqual({
      provider: "buzz-local",
      model: "Qwen/model",
      connection: "connection-fixture",
    });
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { __MODEL_CONNECTION_WRITES__: unknown[] })
            .__MODEL_CONNECTION_WRITES__,
      ),
    )
    .toEqual([
      {
        input: {
          name: "Spark fixture",
          base_url: "http://192.168.1.50:8010/v1",
        },
      },
    ]);
});

test("manual connection failure preserves input, availability retries, and removal is explicit", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(() =>
    Boolean(
      (window as unknown as { __TAURI_INTERNALS__?: { invoke?: unknown } })
        .__TAURI_INTERNALS__?.invoke,
    ),
  );
  await page.evaluate(() => {
    const target = window as unknown as {
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
    };
    const original = target.__TAURI_INTERNALS__.invoke;
    let rows: unknown[] = [];
    let saves = 0;
    let checks = 0;
    const row = {
      id: "manual",
      name: "My compute",
      base_url: "http://computer.local:8000/v1",
      models: ["Qwen/model"],
      has_key: true,
      tested_model: null,
      tested_at: null,
    };
    target.__TAURI_INTERNALS__.invoke = async (command, payload) => {
      if (command === "list_model_connections") return rows;
      if (command === "discover_model_servers")
        return { servers: [], notice: "Fixture scan finished" };
      if (command === "save_model_connection") {
        if (++saves === 1)
          throw new Error("Server is unavailable from this computer");
        rows = [row];
        return row;
      }
      if (command === "check_model_connection") {
        if (++checks === 1) throw new Error("Server disconnected");
        return row.models;
      }
      if (command === "test_model_connection")
        return {
          ...row,
          tested_model: "Qwen/model",
          tested_at: 1,
          qualification: {
            revision: 1,
            model: "Qwen/model",
            context_window: 65536,
            output_token_limit: 8192,
            protocols: [
              {
                protocol: "chat_completions",
                tool_round_trip: true,
                reason: null,
                effort_levels: [],
              },
              {
                protocol: "responses",
                tool_round_trip: false,
                reason:
                  "Server returned HTTP 404. Check the address or API key.",
                effort_levels: [],
              },
              {
                protocol: "anthropic_messages",
                tool_round_trip: true,
                reason: null,
                effort_levels: ["low", "medium", "xhigh"],
              },
            ],
          },
        };
      if (command === "remove_model_connection") {
        rows = [];
        return null;
      }
      return original(command, payload);
    };
  });
  await openSettings(page, "agents");
  await page
    .getByRole("button", { name: "Add connection", exact: true })
    .click();
  const dialog = page.getByRole("dialog", {
    name: "Connect a model server",
    exact: true,
  });
  await dialog
    .getByRole("button", {
      name: "Don’t see your server? Add manually",
      exact: true,
    })
    .click();
  await dialog
    .getByLabel("Connection name", { exact: true })
    .fill("My compute");
  await dialog
    .getByLabel("Server address", { exact: true })
    .fill("computer.local:8000");
  await dialog
    .getByLabel("API key (optional)", { exact: true })
    .fill("fixture-key");
  await dialog
    .getByRole("button", { name: "Save connection", exact: true })
    .click();
  await expect(dialog.getByRole("alert")).toContainText("unavailable");
  await expect(
    dialog.getByLabel("Server address", { exact: true }),
  ).toHaveValue("computer.local:8000");
  await dialog
    .getByRole("button", { name: "Save connection", exact: true })
    .click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByText("My compute", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(
    page.getByText("Available · 1 models", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Test a model", exact: true }).click();
  await page.getByRole("menuitem", { name: "Qwen/model", exact: true }).click();
  await expect(page.getByText(/Compatibility tested: Qwen/)).toBeVisible();
  await expect(
    page.getByText("Chat Completions: Streaming tools and replies passed"),
  ).toBeVisible();
  await expect(
    page.getByText(/Responses: Server returned HTTP 404/),
  ).toBeVisible();
  await expect(page.getByText("Context: 65,536 tokens")).toBeVisible();
  await page.getByRole("button", { name: "Remove", exact: true }).click();
  await expect(page.getByText("My compute", { exact: true })).toBeVisible();
  await page
    .getByRole("button", { name: "Remove connection", exact: true })
    .click();
  await expect(page.getByText("My compute", { exact: true })).toHaveCount(0);
});
