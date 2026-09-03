import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import liveFixture from "../fixtures/agent-usage-live-series.json" with {
  type: "json",
};

/**
 * Renders the Agent usage panel against a **real** `get_agent_usage_series`
 * response.
 *
 * `agent-usage-live-series.json` is not hand-written. It is the verbatim JSON
 * that `archive::agent_usage_series` — the exact function the Tauri command
 * calls — produced when run over a copy of a live `archive.db`, via the
 * `archive::agent_usage_live_evidence` harness. Every token count, session
 * count, channel id, and completeness flag in it came out of real archived
 * kind-44200 payloads and the production accounting ladder.
 *
 * Using it as the fixture means a shape drift between the Rust response and
 * the panel fails this spec instead of being smoothed over by a fixture
 * written to match whatever the panel currently expects.
 *
 * Only the agent display names are seeded on top: the panel resolves a pubkey
 * to a name through the managed-agent list, which the archive does not carry.
 * Channel ids are left unresolved on purpose and render as their real UUID
 * prefix rather than a fabricated name.
 */

const SHOTS = "test-results/agent-usage";

const HELM_PUBKEY =
  "d590bcf4633a5d10f3d2c845d3a1716e752993dc0b83e0c77642affc41dd4701";

async function openAgentUsageSettings(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await expect(page.getByTestId("settings-view")).toBeVisible();
  await page.getByTestId("settings-nav-agent-usage").click();
  const card = page.getByTestId("settings-agent-usage");
  await expect(card).toBeVisible({ timeout: 10_000 });
  return card;
}

async function settleAnimations(el: import("@playwright/test").Locator) {
  await el.evaluate((node) =>
    Promise.all(node.getAnimations({ subtree: true }).map((a) => a.finished)),
  );
}

test.describe("agent usage panel", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test("renders a real archived series without flattening unknowns", async ({
    page,
  }) => {
    await installMockBridge(
      page,
      {
        agentUsageSeries: liveFixture.series,
        managedAgents: liveFixture.agentNames.map((a) => ({
          pubkey: a.pubkey,
          name: a.name,
        })),
      },
      { seedPreviewFeatures: true },
    );

    const card = await openAgentUsageSettings(page);
    const agents = card.getByTestId("agent-usage-agent");
    await expect(agents).toHaveCount(liveFixture.series.agents.length);

    const helm = card.locator(`[data-agent-pubkey="${HELM_PUBKEY}"]`);
    await expect(helm).toBeVisible();

    // The exact figures an independent SQLite query over the same window
    // returns for this agent. Grouped with the same separators the panel uses,
    // so a formatting regression that silently truncates a large counter fails
    // here rather than shipping a wrong number.
    await expect(helm).toContainText("78,270,441");
    await expect(helm).toContainText("76,536,573");
    await expect(helm).toContainText("1,710,735");
    await expect(helm).toContainText("32 turns");

    // Unknown must render as a dash, never as 0. Helm's fresh-input total is
    // underivable in this window (one turn reported no input), so it is the
    // real unknown case in this data, not a synthetic one.
    const freshInput = helm.locator("div", { hasText: /^Fresh input/ }).first();
    await expect(freshInput).toContainText("—");
    await expect(freshInput).not.toContainText("0");

    // A lower-bound total is marked, so it is never read as exact.
    await expect(helm).toContainText("≥");

    // No harness in this archive has published provenance yet, so the panel
    // must say it was not reported rather than claiming zero retries.
    await expect(helm).toContainText("Not reported");
    await expect(card).toContainText("unknown, not zero");

    // Nor may it claim none of these sessions replaced a torn-down one: with
    // no cause reported, that is unknown. This asserts the corrected wording
    // and would fail against the version that stated it as fact.
    await expect(helm).toContainText(
      "No session here reported why it was created",
    );
    await expect(helm).not.toContainText("None of the");

    await settleAnimations(card);
    // Viewport-scoped, not `card.screenshot()`: the card is taller than its
    // scroll container, and an element shot of it paints the clipped remainder
    // as empty canvas — an image that looks like a rendering bug rather than
    // the panel. The per-agent section fits and is captured whole.
    await page.screenshot({ path: `${SHOTS}/01-agent-usage-panel.png` });
    await helm.scrollIntoViewIfNeeded();
    await settleAnimations(card);
    await helm.screenshot({ path: `${SHOTS}/02-helm-detail.png` });
  });
});
