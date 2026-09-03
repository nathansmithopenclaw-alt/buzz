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
 * calls — produced when run over an archive copy whose identifiers had been
 * deterministically anonymized first, via the
 * `archive::agent_usage_live_evidence` harness. So every token count, session
 * count, per-channel split, and completeness flag came out of real archived
 * kind-44200 payloads and the production accounting ladder, while the fixture
 * carries no real identity. The fixture's `_provenance` block holds the exact
 * mapping and the regeneration command.
 *
 * Using it as the fixture means a shape drift between the Rust response and
 * the panel fails this spec instead of being smoothed over by a fixture
 * written to match whatever the panel currently expects.
 *
 * Nothing below is a hardcoded identifier or figure: the spec picks its
 * subject and its expected numbers out of the fixture, so regenerating the
 * fixture cannot leave a stale literal quietly asserting the wrong thing.
 */

const SHOTS = "test-results/agent-usage";

/** The fixture's largest-spend agent — a stable, self-describing choice. */
function largestSpender() {
  const withInput = liveFixture.series.agents.filter(
    (a) => a.usage.inputTokens.value !== null,
  );
  if (withInput.length === 0)
    throw new Error("fixture has no agent with input");
  return withInput.reduce((best, a) =>
    BigInt(a.usage.inputTokens.value as string) >
    BigInt(best.usage.inputTokens.value as string)
      ? a
      : best,
  );
}

/**
 * Group a counter the way the panel does, computed independently of the
 * component's own formatter so this cross-checks grouping rather than
 * restating it.
 */
function grouped(raw: string): string {
  return BigInt(raw).toLocaleString("en-US");
}

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

test.describe("agent usage fixture hygiene", () => {
  /**
   * Every identifier in the fixture must be reproducible from the scheme its
   * `_provenance` block documents.
   *
   * This is the guard that survives upstream: it needs no access to any live
   * archive, and it fails if someone regenerates the fixture from a
   * non-anonymized copy and commits real pubkeys or channel ids. Checking
   * "does this hex match a real agent" would require the private data the
   * check exists to keep out of the tree; checking "is this hex derivable"
   * does not.
   */
  test("every identifier is derivable from the documented anonymization", async () => {
    const { createHash } = await import("node:crypto");
    const sha256 = (label: string) =>
      createHash("sha256").update(label).digest("hex");
    // How far to search for each id's index. Generous — the source archive can
    // hold far more agents and channels than any one window surfaces.
    const MAX_INDEX = 500;

    const agentKeys = new Map<string, number>();
    const channelIds = new Map<string, number>();
    for (let n = 1; n <= MAX_INDEX; n += 1) {
      agentKeys.set(sha256(`fixture-agent-${n}`), n);
      const h = sha256(`fixture-channel-${n}`).slice(0, 32);
      channelIds.set(
        `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20, 32)}`,
        n,
      );
    }

    expect(liveFixture.series.agents.length).toBeGreaterThan(0);
    for (const agent of liveFixture.series.agents) {
      expect(
        agentKeys.has(agent.agentPubkey),
        `agentPubkey ${agent.agentPubkey} is not sha256("fixture-agent-N") — the fixture may carry a real identity`,
      ).toBe(true);
      for (const channel of agent.channels) {
        if (channel.channelId === null) continue;
        expect(
          channelIds.has(channel.channelId),
          `channelId ${channel.channelId} is not a documented synthetic id`,
        ).toBe(true);
      }
    }

    // A display name must carry the same index as the pubkey it labels, so the
    // two can never drift into describing different agents.
    for (const entry of liveFixture.agentNames) {
      const index = agentKeys.get(entry.pubkey);
      expect(
        index,
        `name entry ${entry.pubkey} is not a synthetic key`,
      ).toBeDefined();
      expect(entry.name).toBe(`Agent ${index}`);
    }
  });
});

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

    const subject = largestSpender();
    const row = card.locator(`[data-agent-pubkey="${subject.agentPubkey}"]`);
    await expect(row).toBeVisible();

    // The figures an independent SQLite query over the same window returns,
    // grouped independently of the component's formatter — so a regression
    // that truncates or rounds a counter past 2^53 fails here.
    for (const field of [
      subject.usage.inputTokens,
      subject.usage.cacheReadTokens,
      subject.usage.outputTokens,
    ]) {
      if (field.value !== null) {
        await expect(row).toContainText(grouped(field.value));
      }
    }
    await expect(row).toContainText(
      `${subject.reportCount.toLocaleString("en-US")} turns`,
    );

    // Unknown renders as a dash, never as 0. Assert against whichever field
    // the fixture actually has no value for, so this tracks the real data
    // rather than a case that may not survive a regeneration.
    const unknownField = (
      [
        ["Fresh input", subject.usage.freshInputTokens],
        ["Cache write", subject.usage.cacheWriteTokens],
        ["Output", subject.usage.outputTokens],
      ] as const
    ).find(([, f]) => f.value === null);
    if (unknownField) {
      const [label] = unknownField;
      const cell = row
        .locator("div", { hasText: new RegExp(`^${label}`) })
        .first();
      await expect(cell).toContainText("—");
      await expect(cell).not.toContainText("0");
    }

    // A lower-bound total is marked so it is never read as exact. Scoped to
    // the fields the panel actually renders and to values it actually prints:
    // `totalTokens` is carried in the response but has no tile, and a null
    // value renders as a dash with no marker, so neither can produce a `≥`.
    const anyDisplayedLowerBound = [
      subject.usage.inputTokens,
      subject.usage.outputTokens,
      subject.usage.cacheReadTokens,
      subject.usage.cacheWriteTokens,
      subject.usage.freshInputTokens,
    ].some((f) => f.value !== null && f.incomplete);
    if (anyDisplayedLowerBound) await expect(row).toContainText("≥");

    // No harness in this archive published provenance, so the panel must say
    // it was not reported rather than claiming zero retries...
    expect(subject.provenance.turnsWithAttempt).toBe(0);
    await expect(card).toContainText("unknown, not zero");

    // ...and it must not claim none of these sessions replaced a torn-down
    // one, which with no cause reported is unknown. Asserts the corrected
    // wording; the version that stated it as fact would fail here.
    await expect(row).toContainText(
      "No session here reported why it was created",
    );
    await expect(row).not.toContainText("None of the");

    await settleAnimations(card);
    // Viewport-scoped, not `card.screenshot()`: the card is taller than its
    // scroll container, and an element shot of it paints the clipped remainder
    // as empty canvas — an image that looks like a rendering bug rather than
    // the panel. The per-agent section fits and is captured whole.
    await page.screenshot({ path: `${SHOTS}/01-agent-usage-panel.png` });
    await row.scrollIntoViewIfNeeded();
    await settleAnimations(card);
    await row.screenshot({ path: `${SHOTS}/02-agent-detail.png` });
  });
});
