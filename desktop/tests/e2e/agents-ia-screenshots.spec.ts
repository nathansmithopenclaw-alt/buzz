import { expect, test } from "@playwright/test";

import type { RelayEvent } from "@/shared/api/types";
import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/agents-a2-screenshots";
const OWNER = "deadbeef".repeat(8);
const HOST = "018f42c2-9bc2-7dc1-a41e-2f77769f0142";

function projectionEvent(
  kind: number,
  content: Record<string, unknown>,
  dTag: string,
  createdAt: number,
  idDigit: string,
): RelayEvent {
  return {
    id: idDigit.repeat(64),
    pubkey: OWNER,
    created_at: createdAt,
    kind,
    tags: [["d", dTag]],
    content: JSON.stringify(content),
    sig: "f".repeat(128),
  };
}

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
    await installMockBridge(
      page,
      {
        fleetVisibilitySettings: {
          enabled: false,
          needsChoice: false,
          role: "host",
        },
        managedAgents: [PROOF_AGENT],
        personas: [PROOF_PERSONA],
        fleetProjectionEvents: [
          projectionEvent(
            30177,
            {
              name: "Must Not Render",
              persona_id: null,
              parallelism: 1,
              respond_to: "owner-only",
              respond_to_allowlist: [],
            },
            "4".repeat(64),
            100,
            "1",
          ),
        ],
      },
      { seedPreviewFeatures: false },
    );
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
    await expect(page.getByTestId("fleet-host-status")).toHaveCount(0);
    await expect(
      page.getByText("Must Not Render", { exact: true }),
    ).toHaveCount(0);
    await page.getByTestId("agents-page-content").screenshot({
      path: `${SHOTS}/01-host-experiment-off.png`,
    });
  });

  test("experiment on preserves the local Host IA", async ({ page }) => {
    const now = Math.floor(Date.now() / 1_000);
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
      fleetProjectionEvents: [
        projectionEvent(
          30180,
          {
            v: 1,
            name: "Proof host",
            platform: "macos",
            created_at: now - 60,
          },
          HOST,
          now - 30,
          "2",
        ),
        projectionEvent(
          30181,
          {
            v: 1,
            seq: 1,
            ts: now - 2,
            agents: [
              {
                pubkey: PROOF_AGENT.pubkey,
                state: "running",
                since: now - 30,
              },
            ],
          },
          HOST,
          now - 2,
          "3",
        ),
      ],
    });
    await openAgentsView(page);

    await expect(page.getByTestId("agents-fleet")).toBeVisible();
    await expect(page.getByTestId("agents-library-personas")).toBeVisible();
    await expect(page.getByTestId("agents-library-teams")).toBeVisible();
    await expect(page.getByTestId("fleet-host-status")).toContainText(
      "Proof host",
    );
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

  test("Observer uses the same Fleet, Library, and Teams IA", async ({
    page,
  }) => {
    const now = Math.floor(Date.now() / 1_000);
    const observerAgent = "7".repeat(64);
    await installMockBridge(page, {
      fleetVisibilitySettings: {
        enabled: true,
        hostId: HOST,
        hostName: "Observer device",
        needsChoice: false,
        role: "observer",
      },
      managedAgents: [
        {
          name: "Local Ghost",
          pubkey: "8".repeat(64),
          status: "stopped",
        },
      ],
      personas: [
        {
          id: "local-ghost",
          displayName: "Local Ghost Definition",
          systemPrompt: "Never render on Observer.",
        },
      ],
      fleetProjectionEvents: [
        projectionEvent(
          30177,
          {
            name: "Relay Release Guide",
            persona_id: "relay-release-guide",
            parallelism: 1,
            respond_to: "owner-only",
            respond_to_allowlist: [],
          },
          observerAgent,
          now - 20,
          "4",
        ),
        projectionEvent(
          30175,
          {
            display_name: "Relay Release Guide",
            system_prompt: "Display fields only.",
            avatar_url: null,
            runtime: "codex",
            model: "gpt-5",
            provider: null,
            name_pool: [],
          },
          "relay-release-guide",
          now - 19,
          "5",
        ),
        projectionEvent(
          30180,
          {
            v: 1,
            name: "Mini host",
            platform: "macos",
            created_at: now - 60,
          },
          HOST,
          now - 18,
          "6",
        ),
        projectionEvent(
          30181,
          {
            v: 1,
            seq: 1,
            ts: now - 2,
            agents: [
              { pubkey: observerAgent, state: "running", since: now - 20 },
            ],
          },
          HOST,
          now - 2,
          "7",
        ),
      ],
    });
    await openAgentsView(page);

    await expect(page.getByTestId("agents-fleet")).toContainText(
      "Relay Release Guide",
    );
    await expect(page.getByTestId("agents-library-personas")).toContainText(
      "Relay Release Guide",
    );
    await expect(page.getByTestId("agents-library-teams")).toBeVisible();
    await expect(page.getByText("Local Ghost", { exact: true })).toHaveCount(0);
    await expect(page.getByTestId("new-agent-card")).toHaveCount(0);
    await page.getByTestId("agents-page-content").screenshot({
      path: `${SHOTS}/04-observer-experiment-on.png`,
    });
  });
});
