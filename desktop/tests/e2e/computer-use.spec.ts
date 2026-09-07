import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";
import { BrowserService } from "../../../computer-use/src/browser.mjs";
import { startViewer } from "../../../computer-use/src/viewer.mjs";

test("Buzz panel displays and hands back the agent's actual isolated browser", async ({
  page,
}, testInfo) => {
  const service = new BrowserService();
  const viewer = await startViewer(service);
  try {
    const session = await service.open();
    await service
      .get(session.id)
      .page.setContent(
        '<title>Buzz browser acceptance</title><main style="font:24px system-ui;padding:50px"><h1>Shared page, isolated session</h1><label>Verification <input value="Agent opened this page"></label></main>',
      );
    await page.setViewportSize({ width: 1440, height: 1000 });
    await installMockBridge(page);
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await expect(page.getByTestId("app-sidebar")).toBeVisible();
    await page.evaluate((viewerUrl) => {
      const internals = (
        window as unknown as {
          __TAURI_INTERNALS__: {
            invoke: (command: string, args: unknown) => Promise<unknown>;
          };
        }
      ).__TAURI_INTERNALS__;
      const original = internals.invoke;
      internals.invoke = async (command: string, args: unknown) => {
        if (command === "computer_use_status")
          return {
            enabled: true,
            dependencyReady: true,
            nativeEnabled: false,
            nativeReady: true,
            detail: "Local browser tools are enabled for new agent sessions.",
          };
        if (command === "list_computer_browser_sessions")
          return [
            {
              id: "test-connection",
              owner: "Acceptance agent",
              label: "Acceptance agent",
              viewerUrl,
              created: new Date().toISOString(),
            },
          ];
        return original(command, args);
      };
    }, viewer.url);
    await page.getByRole("button", { name: "Open agent browsers" }).click();
    const frame = page.frameLocator('iframe[title="Live agent browser"]');
    await expect(frame.getByRole("img")).toBeVisible();
    await frame
      .getByRole("button", { name: "Take control", exact: true })
      .click();
    await expect(
      frame.getByRole("button", { name: "Return to agent", exact: true }),
    ).toBeVisible();
    expect(service.get(session.id).controller).toBe("human");
    await waitForAnimations(page);
    await page.screenshot({
      path: testInfo.outputPath("buzz-browser-human-control.png"),
    });
    await frame
      .getByRole("button", { name: "Return to agent", exact: true })
      .click();
    await expect(
      frame.getByRole("button", { name: "Take control", exact: true }),
    ).toBeVisible();
    expect(service.get(session.id).controller).toBe("agent");
    await page
      .getByRole("dialog")
      .getByRole("button", { name: "Close", exact: true })
      .click();
    await expect(page.getByRole("dialog")).toBeHidden();
  } finally {
    await viewer.close();
    await service.shutdown();
  }
});
