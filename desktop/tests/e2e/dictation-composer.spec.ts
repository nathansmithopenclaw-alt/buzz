import { expect, test, type Page } from "@playwright/test";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { FEATURE_OVERRIDES_STORAGE_KEY } from "../helpers/features";

type Fixture = {
  transcript: string;
  polish?: { text: string; delayMs: number };
  /** Alice is a managed agent named Grok instead of a person. */
  agent?: boolean;
};

/**
 * Dictation into the real composer: only the native boundary (microphone,
 * Web Audio, speech engine, polish model) is faked.
 */
async function setup(page: Page, fixture: Fixture) {
  await page.addInitScript(
    ({ key, fixture }) => {
      localStorage.setItem(key, JSON.stringify({ dictation: true }));
      window.__BUZZ_E2E_DICTATION__ = {
        ...fixture,
        polishRequests: [],
        saved: {
          version: 2,
          speech: { kind: "local", modelId: "parakeet-tdt-0.6b-v2-int8" },
          cleanup: { kind: "agent_open_router", model: "m" },
          timeoutMs: 4000,
          hotkey: "AltRight",
          language: "en",
          cloudCleanupEnabled: true,
        },
      };
      Object.defineProperty(navigator, "mediaDevices", {
        value: {
          enumerateDevices: async () => [],
          getUserMedia: async () => ({ getTracks: () => [{ stop() {} }] }),
          addEventListener() {},
          removeEventListener() {},
        },
      });
      class FakeAudioContext {
        state = "running";
        sampleRate = 48_000;
        audioWorklet = { addModule: async () => {} };
        createMediaStreamSource() {
          return { connect() {}, disconnect() {} };
        }
        resume() {
          return Promise.resolve();
        }
        close() {
          return Promise.resolve();
        }
      }
      class FakeAudioWorkletNode {
        port = { onmessage: null, postMessage() {} };
        connect() {}
        disconnect() {}
      }
      Object.assign(window, {
        AudioContext: FakeAudioContext,
        AudioWorkletNode: FakeAudioWorkletNode,
      });
    },
    { key: FEATURE_OVERRIDES_STORAGE_KEY, fixture },
  );
  const pubkey = TEST_IDENTITIES.alice.pubkey;
  await installMockBridge(page, {
    managedAgents: fixture.agent
      ? [{ pubkey, name: "Grok", status: "running", channelNames: ["general"] }]
      : [],
    searchProfiles: [
      fixture.agent
        ? { pubkey, displayName: "Grok", isAgent: true }
        : { pubkey, displayName: "Alice Chen" },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

async function dictate(page: Page) {
  const button = page
    .getByTestId("message-composer")
    .getByTestId("composer-dictation-button");
  await button.click();
  await expect(button).toHaveAttribute("aria-pressed", "true");
  await button.click();
}

test.describe("Composer dictation", () => {
  test("words and mentions land at once; polish tidies them in place", async ({
    page,
  }) => {
    await setup(page, {
      transcript: "Alice Chen, um, can you check the the deploy logs",
      polish: {
        text: "{{1}}, can you check the deploy logs?",
        delayMs: 1500,
      },
    });
    const input = page.getByTestId("message-input");
    await dictate(page);
    await expect(input).toHaveText(
      "@Alice Chen, can you check the the deploy logs",
    );
    await expect(page.getByTestId("send-message")).toBeEnabled();
    await expect(input).toHaveText(
      "@Alice Chen, can you check the deploy logs?",
    );
    expect(
      await page.evaluate(() => window.__BUZZ_E2E_DICTATION__?.polishRequests),
    ).toEqual(["{{1}}, can you check the the deploy logs"]);
  });

  test("polish never overwrites words edited while it ran", async ({
    page,
  }) => {
    await setup(page, {
      transcript: "Alice Chen, can you check the the deploy logs",
      polish: {
        text: "{{1}}, can you check the deploy logs?",
        delayMs: 1500,
      },
    });
    const input = page.getByTestId("message-input");
    await dictate(page);
    await expect(input).toHaveText(
      "@Alice Chen, can you check the the deploy logs",
    );
    await input.click();
    await page.keyboard.press("End");
    await page.keyboard.press("Backspace");
    await page.waitForTimeout(2000);
    await expect(input).toHaveText(
      "@Alice Chen, can you check the the deploy log",
    );
  });

  test("a second dictation in the same composer is polished too", async ({
    page,
  }) => {
    await setup(page, {
      transcript: "Alice Chen, can you check the the deploy logs",
      polish: {
        text: "{{1}}, can you check the deploy logs?",
        delayMs: 800,
      },
    });
    const input = page.getByTestId("message-input");
    await dictate(page);
    await expect(input).toHaveText(
      "@Alice Chen, can you check the deploy logs?",
    );
    await input.click();
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.press("Backspace");
    await expect(input).toHaveText("");
    await dictate(page);
    await expect(input).toHaveText(
      "@Alice Chen, can you check the the deploy logs",
    );
    await expect(input).toHaveText(
      "@Alice Chen, can you check the deploy logs?",
    );
  });

  test("dictating with the hotkey while typing in the composer is polished", async ({
    page,
  }) => {
    await setup(page, {
      transcript: "Alice Chen, can you check the the deploy logs",
      polish: {
        text: "{{1}}, can you check the deploy logs?",
        delayMs: 800,
      },
    });
    const input = page.getByTestId("message-input");
    const button = page.getByTestId("composer-dictation-button");
    await input.click();
    await page.keyboard.press("AltRight");
    await expect(button).toHaveAttribute("aria-pressed", "true");
    await page.keyboard.press("AltRight");
    await expect(input).toHaveText(
      "@Alice Chen, can you check the the deploy logs",
    );
    await expect(input).toHaveText(
      "@Alice Chen, can you check the deploy logs?",
    );
  });

  test("an agent mention is polished too", async ({ page }) => {
    await setup(page, {
      agent: true,
      transcript: "Grok, can you check the the deploy logs",
      polish: {
        text: "{{1}}, can you check the deploy logs?",
        delayMs: 800,
      },
    });
    const input = page.getByTestId("message-input");
    await dictate(page);
    await expect(input).toHaveText("@Grok, can you check the the deploy logs");
    await expect(input).toHaveText("@Grok, can you check the deploy logs?");
  });
});
