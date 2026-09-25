import { expect, test, type Page } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";
import { waitForAnimations } from "../helpers/animations";

async function setup(page: Page, lowSpace = false, platform = "MacIntel") {
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
  await installMockBridge(page);
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

test.describe("Dictation settings", () => {
  test.use({ viewport: { width: 1280, height: 1000 } });

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
        name: "Gemini 3.8 Flash · Recommended",
        exact: true,
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
