import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/agents-a1-screenshots";

async function openAgentsView(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-page-content")).toBeVisible();
  await waitForAnimations(page);
}

const PROOF_PERSONA = {
  id: "custom:release-guide",
  displayName: "Release Guide",
  runtime: "codex",
  systemPrompt: "Guide a release carefully.",
};

const PROOF_AGENT = {
  name: "Release Guide",
  personaId: PROOF_PERSONA.id,
  pubkey: TEST_IDENTITIES.alice.pubkey,
  status: "running" as const,
};

test.describe("Agents Host IA screenshots", () => {
  test.use({ viewport: { height: 960, width: 1280 } });

  test("experiment off shows roster, template shelf, and teams", async ({
    page,
  }) => {
    await installMockBridge(page, {
      fleetVisibilitySettings: {
        enabled: false,
        needsChoice: false,
        role: "host",
      },
      managedAgents: [PROOF_AGENT],
      personas: [PROOF_PERSONA],
    });
    await openAgentsView(page);

    await expect(page.getByTestId("agents-fleet")).toContainText(
      "Release Guide",
    );
    await expect(page.getByTestId("agents-fleet")).toContainText(
      "codex · Default model",
    );
    await expect(page.getByTestId("agents-library-personas")).toContainText(
      "Deployed as →",
    );
    await page.getByTestId("agents-page-content").screenshot({
      path: `${SHOTS}/01-host-experiment-off.png`,
    });
  });

  test("experiment on preserves the local Host IA", async ({ page }) => {
    await installMockBridge(page, {
      fleetVisibilitySettings: {
        enabled: true,
        hostId: "host-a1-proof",
        hostName: "Proof host",
        needsChoice: false,
        role: "host",
      },
      managedAgents: [PROOF_AGENT],
      personas: [PROOF_PERSONA],
    });
    await openAgentsView(page);

    await expect(page.getByTestId("agents-fleet")).toBeVisible();
    await expect(page.getByTestId("agents-library-personas")).toBeVisible();
    await expect(page.getByTestId("agents-library-teams")).toBeVisible();
    await page.getByTestId("agents-page-content").screenshot({
      path: `${SHOTS}/02-host-experiment-on.png`,
    });
  });

  test("fresh profile shows one onboarding state", async ({ page }) => {
    await installMockBridge(page, {
      activePersonaIds: [],
      fleetVisibilitySettings: {
        enabled: false,
        needsChoice: false,
        role: "host",
      },
      managedAgents: [],
      personas: [],
      teams: [],
    });
    await openAgentsView(page);

    await expect(page.getByTestId("agents-empty-state")).toContainText(
      "Create your first agent",
    );
    await page.getByTestId("agents-page-content").screenshot({
      path: `${SHOTS}/03-fresh-profile.png`,
    });
  });
});
