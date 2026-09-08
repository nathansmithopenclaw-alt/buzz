import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

// UI/IPC fixture proof; endpoint discovery and the dsh launcher have separate real tests.
test("DeepSeek offers explicit providers and sends the local endpoint with model discovery", async ({
  page,
}) => {
  await installMockBridge(page, {
    globalAgentConfig: {
      preferred_runtime: "deepseek",
      provider: null,
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
        provider_options: [
          ["deepseek-hosted", "DeepSeek API"],
          ["deepseek-compatible", "Local / OpenAI-compatible endpoint"],
        ],
        source: "preset",
        install_hint: "",
        can_auto_install: false,
      },
    ],
    discoverAgentModels: {
      models: [{ id: "Qwen/Qwen3.8-27B", name: "Qwen/Qwen3.8-27B" }],
      supportsSwitching: true,
    },
  });
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("tab", { name: "Customize for this agent" }).click();
  const provider = dialog.getByLabel("LLM provider", { exact: true });
  await expect(provider).toBeVisible();
  await provider.selectOption("deepseek-hosted");
  await expect(
    dialog.getByLabel("DeepSeek API key", { exact: true }),
  ).toBeVisible();
  await expect(
    dialog.getByLabel("Model endpoint URL", { exact: true }),
  ).toHaveCount(0);
  await provider.selectOption("deepseek-compatible");
  await dialog
    .getByLabel("Model endpoint URL", { exact: true })
    .fill("http://127.0.0.1:8010/v1");
  await expect(dialog.getByLabel("Endpoint API key (optional)")).toBeVisible();
  const model = dialog.getByRole("combobox", { name: "Model", exact: true });
  await model.click();
  await page.getByRole("button", { name: /Qwen/ }).click();
  await expect(model).toContainText("Qwen");
  await expect
    .poll(() =>
      page.evaluate(() => {
        const log =
          (
            window as unknown as {
              __BUZZ_E2E_COMMAND_LOG__?: Array<{
                command: string;
                payload?: unknown;
              }>;
            }
          ).__BUZZ_E2E_COMMAND_LOG__ ?? [];
        return log.some(
          (entry) =>
            entry.command === "discover_agent_models" &&
            JSON.stringify(entry.payload).includes("http://127.0.0.1:8010/v1"),
        );
      }),
    )
    .toBe(true);
  await dialog
    .getByLabel("Agent name", { exact: true })
    .fill("DeepSeek fixture");
  await dialog
    .getByLabel("Agent instructions", { exact: true })
    .fill("Run only fixture tasks.");
  await dialog.getByRole("button", { name: "Add agent", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const log =
          (
            window as unknown as {
              __BUZZ_E2E_COMMAND_LOG__?: Array<{
                command: string;
                payload?: {
                  input?: {
                    model?: string;
                    provider?: string;
                    envVars?: Record<string, string>;
                  };
                };
              }>;
            }
          ).__BUZZ_E2E_COMMAND_LOG__ ?? [];
        const saved = log.find((entry) => entry.command === "create_persona")
          ?.payload?.input;
        return (
          saved && {
            model: saved.model,
            provider: saved.provider,
            endpoint: saved.envVars?.DSH_LLM_BASE_URL,
          }
        );
      }),
    )
    .toEqual({
      model: "Qwen/Qwen3.8-27B",
      provider: "deepseek-compatible",
      endpoint: "http://127.0.0.1:8010/v1",
    });
});
