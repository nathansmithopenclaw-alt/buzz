import { expect, test, type Locator, type Page } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";
import { waitForAnimations } from "../helpers/animations";

async function setup(
  page: Page,
  lowSpace = false,
  platform = "MacIntel",
  claudeCommand?: string,
) {
  await page.addInitScript(
    ({ lowSpace, platform }) => {
      window.__BUZZ_E2E_DICTATION__ = { lowSpace };
      Object.defineProperty(navigator, "platform", { get: () => platform });
      let revealed = false;
      Object.defineProperty(navigator, "mediaDevices", {
        value: {
          enumerateDevices: async () =>
            revealed
              ? [
                  {
                    kind: "audioinput",
                    deviceId: "usb",
                    label: "USB microphone",
                  },
                ]
              : [],
          getUserMedia: async () => {
            revealed = true;
            return { getTracks: () => [{ stop() {} }] };
          },
          addEventListener() {},
          removeEventListener() {},
        },
      });
    },
    { lowSpace, platform },
  );
  await installMockBridge(
    page,
    claudeCommand
      ? {
          acpRuntimesCatalog: [
            {
              id: "claude",
              label: "Claude Code",
              avatar_url: "",
              availability: "available",
              command: claudeCommand,
              binary_path: claudeCommand,
              default_args: ["--test-adapter-argument"],
              mcp_command: null,
              install_hint: "",
              install_instructions_url: "",
              can_auto_install: false,
              auth_status: { status: "authenticated" },
            },
          ],
        }
      : {},
  );
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "voice");
  await expect(page.getByTestId("dictation-settings")).toBeVisible();
}

async function shot(page: Page, name: string) {
  await waitForAnimations(page);
  await page.screenshot({
    path: `test-results/dictation-settings/${name}.png`,
  });
}

/** Composite computed CSS colors (including alpha/OKLCH) before WCAG luminance. */
async function contrast(locator: Locator, pseudo?: string) {
  return locator.evaluate((element, pseudo) => {
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 1;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("Canvas unavailable for contrast measurement");
    const ancestors: Element[] = [];
    for (let node: Element | null = element; node; node = node.parentElement)
      ancestors.unshift(node);
    ctx.fillStyle = "white";
    ctx.fillRect(0, 0, 1, 1);
    for (const node of ancestors) {
      ctx.fillStyle = getComputedStyle(node).backgroundColor;
      ctx.fillRect(0, 0, 1, 1);
    }
    const luminance = () => {
      const rgb = [...ctx.getImageData(0, 0, 1, 1).data]
        .slice(0, 3)
        .map((c) => {
          const s = c / 255;
          return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
        });
      return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
    };
    const background = luminance();
    const style = getComputedStyle(element, pseudo);
    ctx.globalAlpha = Number(style.opacity);
    ctx.fillStyle = style.color;
    ctx.fillRect(0, 0, 1, 1);
    const foreground = luminance();
    return (
      (Math.max(background, foreground) + 0.05) /
      (Math.min(background, foreground) + 0.05)
    );
  }, pseudo);
}

test.describe("Dictation settings", () => {
  test.use({ viewport: { width: 1280, height: 1000 } });

  test("discovers Claude cleanup models through the runtime catalog adapter", async ({
    page,
  }) => {
    await setup(page, false, "MacIntel", "/test/claude-code-acp");
    await page.getByLabel("Cleanup model", { exact: true }).click();
    await expect(
      page.getByRole("option", { name: "Claude Haiku 4.5", exact: true }),
    ).toBeVisible();
    const calls = await page.evaluate(() =>
      window.__BUZZ_E2E_COMMAND_LOG__?.filter(
        (call) => call.command === "discover_agent_models",
      ),
    );
    expect(calls).toContainEqual(
      expect.objectContaining({
        payload: {
          input: {
            agentCommand: "/test/claude-code-acp",
            agentArgs: ["--test-adapter-argument"],
            definitionEnv: {},
          },
        },
      }),
    );
  });

  test("does not launch the underlying CLI when the Claude adapter is missing", async ({
    page,
  }) => {
    await setup(page);
    await page.getByLabel("Cleanup model", { exact: true }).click();
    await expect(
      page.getByRole("listbox").getByText("Claude plan", { exact: true }),
    ).toBeVisible();
    expect(
      await page.evaluate(() =>
        window.__BUZZ_E2E_COMMAND_LOG__?.filter(
          (call) => call.command === "discover_agent_models",
        ),
      ),
    ).toEqual([]);
  });

  test("keeps popup headings, badges, metadata and search legible in dark mode", async ({
    page,
  }, testInfo) => {
    await page.addInitScript(() => {
      localStorage.setItem("buzz-theme", "buzz-dark");
      localStorage.setItem("buzz-follow-system", "false");
    });
    await setup(page);
    await expect(page.locator("html")).toHaveClass(/dark/);
    await page.getByLabel("Speech model", { exact: true }).click();
    const speech = page.getByRole("listbox");
    const ratios: Record<string, number> = {
      metadata: await contrast(speech.getByText(/English · .* on disk/)),
      speechBadge: await contrast(
        speech.getByText("Recommended", { exact: true }),
      ),
    };
    await shot(page, "speech-dark-contrast");
    await page.keyboard.press("Escape");
    await page.getByLabel("Cleanup model", { exact: true }).click();
    const cleanup = page.getByRole("listbox");
    ratios.heading = await contrast(
      cleanup.getByText("OpenRouter", { exact: true }),
    );
    ratios.cleanupBadge = await contrast(
      cleanup.getByText("Recommended", { exact: true }),
    );
    const search = cleanup.getByRole("textbox", { name: "Search models" });
    ratios.placeholder = await contrast(search, "::placeholder");
    await search.fill("Gemini");
    ratios.search = await contrast(search);
    await shot(page, "cleanup-dark-contrast");
    await testInfo.attach("computed-contrast", {
      body: JSON.stringify(ratios, null, 2),
      contentType: "application/json",
    });
    for (const [name, ratio] of Object.entries(ratios))
      expect
        .soft(ratio, `${name} contrast against popup surface`)
        .toBeGreaterThanOrEqual(4.5);
  });

  test("keeps only the experiment toggle and puts shared native controls in Voice", async ({
    page,
  }) => {
    await setup(page);
    await expect(
      page.getByTestId("dictation-settings").locator("select"),
    ).toHaveCount(0);
    await expect(
      page.getByLabel("Speech model", { exact: true }),
    ).toHaveAttribute("role", "combobox");
    await expect(
      page.getByText("Cleanup timeout", { exact: true }),
    ).toBeHidden();
    await expect(page.getByText("Cloud cleanup", { exact: true })).toBeHidden();
    await shot(page, "voice-default");
    await page.getByTestId("settings-nav-experimental").click();
    await expect(page.getByTestId("dictation-settings")).toBeHidden();
    await expect(
      page.getByText("Configure in Settings → Voice.", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("Verified on Apple Silicon Macs.", { exact: false }),
    ).toBeHidden();
    await page
      .getByText("Configure in Settings → Voice.", { exact: true })
      .scrollIntoViewIfNeeded();
    await shot(page, "experiments-toggle");
  });

  test("selects a speech model, shows its download and supports removal", async ({
    page,
  }) => {
    await setup(page);
    await page.getByLabel("Speech model", { exact: true }).click();
    await expect(
      page.getByRole("option", { name: /v2.*Recommended/ }),
    ).toBeVisible();
    await shot(page, "speech-catalog");
    await page.getByRole("option", { name: /v3/ }).click();
    await page.getByRole("button", { name: "Download", exact: true }).click();
    await expect(
      page.getByRole("status").filter({ hasText: "Downloading… 42%" }),
    ).toBeVisible();
    await shot(page, "speech-download");
    await page.getByLabel("Speech model", { exact: true }).click();
    await page.getByRole("option", { name: /v2/ }).click();
    await page.getByRole("button", { name: "Remove", exact: true }).click();
    await expect(
      page.getByRole("button", { name: "Download", exact: true }),
    ).toBeVisible();
    const calls = await page.evaluate(() =>
      window.__BUZZ_E2E_COMMAND_LOG__?.filter(
        (call) => call.command === "save_dictation_settings",
      ),
    );
    expect(calls?.[0].payload).toMatchObject({
      settings: {
        speech: { kind: "local", modelId: "parakeet-tdt-0.6b-v3-int8" },
      },
    });
  });

  test("refuses a download when space is insufficient", async ({ page }) => {
    await setup(page, true);
    await page.getByLabel("Speech model", { exact: true }).click();
    await page.getByRole("option", { name: /v3/ }).click();
    await expect(
      page.getByRole("button", { name: "Download", exact: true }),
    ).toBeDisabled();
    await expect(
      page.getByRole("alert").filter({ hasText: "Not enough space" }),
    ).toBeVisible();
    await shot(page, "speech-disk-refusal");
    expect(
      await page.evaluate(() =>
        window.__BUZZ_E2E_COMMANDS__?.includes("dictation_download_model"),
      ),
    ).toBe(false);
  });

  test("groups cleanup sources, saves local connections and keeps custom model control", async ({
    page,
  }) => {
    await setup(page);
    const picker = page.getByLabel("Cleanup model", { exact: true });
    await picker.click();
    const list = page.getByRole("listbox");
    for (const group of [
      "OpenRouter",
      "Model connections",
      "Local models",
      "Claude plan",
    ])
      await expect(list.getByText(group, { exact: true })).toBeVisible();
    await expect(
      page.getByRole("option", {
        name: /Gemini 3.8 Flash.*Recommended/,
      }),
    ).toBeVisible();
    await expect(
      page.getByRole("option", {
        name: "Spark 1 · Qwen · Qwen 3.8 27B",
        exact: true,
      }),
    ).toBeVisible();
    await shot(page, "cleanup-groups");
    await page
      .getByRole("option", {
        name: "Spark 1 · Qwen · Qwen 3.8 27B",
        exact: true,
      })
      .scrollIntoViewIfNeeded();
    await shot(page, "cleanup-local-models");
    await page
      .getByRole("option", {
        name: "Spark 1 · Qwen · Qwen 3.8 27B",
        exact: true,
      })
      .click();
    await expect(
      page.getByText("Cleanup timeout", { exact: true }),
    ).toBeVisible();
    await expect(page.getByText("Cloud cleanup", { exact: true })).toBeHidden();
    const calls = await page.evaluate(() =>
      window.__BUZZ_E2E_COMMAND_LOG__?.filter(
        (call) => call.command === "save_dictation_settings",
      ),
    );
    expect(calls?.at(-1)?.payload).toMatchObject({
      settings: {
        cleanup: {
          kind: "connection",
          connectionId: "spark",
          model: "Qwen 3.8 27B",
        },
      },
    });
    await picker.click();
    await page
      .getByRole("option", { name: "My server · Other model…", exact: true })
      .click();
    await page.getByLabel("Custom cleanup model ID").fill("my-unlisted-model");
    await page.getByRole("button", { name: "Use model", exact: true }).click();
    await expect(picker).toContainText("my-unlisted-model");
    await expect(
      page.getByText("Cloud cleanup", { exact: true }),
    ).toBeVisible();
  });

  test("captures a key without starting dictation and rejects conflicts", async ({
    page,
  }) => {
    await setup(page);
    const key = page.getByRole("button", { name: "Change dictation key" });
    await key.click();
    await expect(key).toHaveText("Press a key…");
    await shot(page, "key-prompt");
    await page.keyboard.press("a");
    await expect(page.getByRole("alert")).toContainText("conflicts");
    await shot(page, "key-conflict");
    await page.keyboard.press("F8");
    await expect(key).toHaveText("F8");
    await shot(page, "key-accepted");
    await key.click();
    await page.keyboard.press("AltRight");
    await expect(key).toHaveText("Right Option");
    expect(
      await page.evaluate(() =>
        window.__BUZZ_E2E_COMMANDS__?.includes("dictation_start"),
      ),
    ).toBe(false);
  });

  test("ignores Windows AltGr and reveals microphones through the native picker", async ({
    page,
  }) => {
    await setup(page, false, "Win32");
    const key = page.getByRole("button", { name: "Change dictation key" });
    await expect(key).toHaveText("Right Alt");
    await key.click();
    await page.evaluate(() => {
      for (const type of ["keydown", "keyup"])
        window.dispatchEvent(
          new KeyboardEvent(type, {
            code: "AltRight",
            key: "AltGraph",
            ctrlKey: true,
            altKey: true,
            bubbles: true,
          }),
        );
    });
    await expect(key).toHaveText("Press a key…");
    await expect(page.getByRole("alert")).toBeHidden();
    await page.keyboard.press("Escape");
    await shot(page, "microphone-find");
    await page.getByRole("button", { name: "Find microphones" }).click();
    await page.getByLabel("Microphone", { exact: true }).click();
    await expect(
      page.getByRole("option", { name: "System default", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("option", { name: "USB microphone", exact: true }),
    ).toBeVisible();
    await shot(page, "microphone-picker");
    await page
      .getByRole("option", { name: "USB microphone", exact: true })
      .click();
    await expect(page.getByLabel("Microphone", { exact: true })).toContainText(
      "USB microphone",
    );
  });
});
