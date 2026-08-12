import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test("Dispatch experiment enables the sidebar route shell", async ({
  page,
}) => {
  await installMockBridge(page, undefined, { seedPreviewFeatures: false });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  await expect(page.getByTestId("open-dispatch-view")).toHaveCount(0);
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-experimental").click();
  await page.getByTestId("feature-toggle-dispatch").click();
  await page.getByTestId("settings-back-to-app").click();
  await expect(page.getByTestId("open-dispatch-view")).toBeVisible();

  await page.getByTestId("open-dispatch-view").click();
  await expect(page).toHaveURL(/\/dispatch$/);
  await expect(page.getByTestId("dispatch-screen")).toBeVisible();
  await expect(page.getByTestId("dispatch-board-mount")).toHaveCount(1);
});
