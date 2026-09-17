import { fileURLToPath } from "node:url";
import { defineConfig, devices } from "@playwright/test";

import { PREVIEW_ORIGIN, PREVIEW_PORT } from "./tests/e2e/previewOrigin";

export default defineConfig({
  globalSetup: fileURLToPath(
    new URL("../scripts/require-detached-browser.mjs", import.meta.url),
  ),
  testDir: "./tests/e2e",
  timeout: 60_000,
  retries: 0,
  workers: 1,
  reporter: [["list"]],
  use: { baseURL: PREVIEW_ORIGIN },
  projects: [
    {
      name: "perf",
      testMatch: ["**/*.perf.ts"],
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    command: `python3 -m http.server ${PREVIEW_PORT} -d dist`,
    cwd: ".",
    reuseExistingServer: true,
    url: PREVIEW_ORIGIN,
  },
});
