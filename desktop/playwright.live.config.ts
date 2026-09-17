import { fileURLToPath } from "node:url";
import { defineConfig } from "@playwright/test";

export default defineConfig({
  globalSetup: fileURLToPath(
    new URL("../scripts/require-detached-browser.mjs", import.meta.url),
  ),
  testDir: "./tests/e2e",
  testMatch: "**/agents-everywhere.live.spec.ts",
  timeout: 90_000,
  workers: 1,
  reporter: "list",
});
