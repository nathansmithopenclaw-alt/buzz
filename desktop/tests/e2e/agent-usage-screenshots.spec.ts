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

type UsageField = { value: string | null; incomplete: boolean };
type CostField = { value: number | null; incomplete: boolean };
type ReportedUsage = Record<string, UsageField> & {
  estimatedCostUsd: CostField;
};

const TOKEN_FIELDS = [
  "inputTokens",
  "outputTokens",
  "totalTokens",
  "cacheReadTokens",
  "cacheWriteTokens",
] as const;

/**
 * Fold finished `ReportedUsage` values the way the Rust accumulator does:
 * each counter is the sum of the known contributions, `incomplete` is OR-ed
 * across them, and `freshInputTokens` is DERIVED from the folded input and
 * cache figures rather than summed — summing per-contributor fresh-input
 * values would be a different (and wrong) quantity.
 *
 * Written here from the accounting rules, not lifted from the backend, so it
 * cross-checks the response instead of restating it.
 */
function foldUsage(usages: ReportedUsage[]) {
  const folded: Record<string, UsageField> = {};
  for (const field of TOKEN_FIELDS) {
    let value: bigint | null = null;
    let incomplete = false;
    for (const usage of usages) {
      const f = usage[field];
      if (f.incomplete) incomplete = true;
      if (f.value !== null) value = (value ?? 0n) + BigInt(f.value);
    }
    folded[field] = {
      value: value === null ? null : String(value),
      incomplete,
    };
  }

  let cost: number | null = null;
  let costIncomplete = false;
  for (const usage of usages) {
    const f = usage.estimatedCostUsd;
    if (f.incomplete) costIncomplete = true;
    if (f.value !== null) cost = (cost ?? 0) + f.value;
  }

  const input = folded.inputTokens;
  const cacheRead = folded.cacheReadTokens;
  const cacheWrite = folded.cacheWriteTokens;
  let fresh: UsageField;
  if (input.incomplete) {
    fresh = { value: null, incomplete: true };
  } else if (input.value === null) {
    fresh = { value: null, incomplete: false };
  } else if (cacheRead.incomplete || cacheWrite.incomplete) {
    fresh = { value: null, incomplete: true };
  } else {
    const subsets =
      BigInt(cacheRead.value ?? "0") + BigInt(cacheWrite.value ?? "0");
    const total = BigInt(input.value);
    fresh =
      subsets <= total
        ? { value: String(total - subsets), incomplete: false }
        : { value: null, incomplete: true };
  }

  return {
    usage: {
      inputTokens: folded.inputTokens,
      outputTokens: folded.outputTokens,
      totalTokens: folded.totalTokens,
      estimatedCostUsd: { value: cost, incomplete: costIncomplete },
      cacheReadTokens: folded.cacheReadTokens,
      cacheWriteTokens: folded.cacheWriteTokens,
      freshInputTokens: fresh,
    },
    hasUnknownUsage:
      TOKEN_FIELDS.some((f) => folded[f].incomplete) ||
      costIncomplete ||
      fresh.incomplete,
  };
}

/** Every usage field reported as unknown — the shape the backend emits for a
 * combined total it refuses to state. */
function blankedUsage() {
  const unknown = { value: null, incomplete: true };
  return {
    inputTokens: unknown,
    outputTokens: unknown,
    totalTokens: unknown,
    estimatedCostUsd: { value: null, incomplete: true },
    cacheReadTokens: unknown,
    cacheWriteTokens: unknown,
    freshInputTokens: unknown,
  };
}

/**
 * A mixed-basis response built out of the live fixture's own agents: half of
 * them declare `per_turn_sum`, the rest `per_response`.
 *
 * The grouping is synthetic because no captured window in the tree mixes bases
 * yet, but every figure inside it is a real archived number folded by the rules
 * above, and the response shape is the one the backend emits for a mixed
 * window: combined `buckets` blanked, `basisTotals` ordered by basis name, and
 * `agents` ordered by identity rather than ranked by volume.
 */
function mixedBasisSeries() {
  const agents = [...liveFixture.series.agents].sort((a, b) =>
    a.agentPubkey.localeCompare(b.agentPubkey),
  );
  const split = Math.ceil(agents.length / 2);
  // Ascending basis name, matching the backend's ordering.
  const groups = [
    { basis: "per_response", members: agents.slice(split) },
    { basis: "per_turn_sum", members: agents.slice(0, split) },
  ];
  if (groups.some((g) => g.members.length === 0))
    throw new Error("fixture has too few agents to split across two bases");

  return {
    ...liveFixture.series,
    agents,
    buckets: liveFixture.series.buckets.map((b) => ({
      ...b,
      usage: blankedUsage(),
      hasUnknownUsage: true,
    })),
    bases: groups.map((g) => g.basis),
    basesMixed: true,
    basisTotals: groups.map((g) => {
      const folded = foldUsage(g.members.map((a) => a.usage as ReportedUsage));
      return {
        basis: g.basis,
        usage: folded.usage,
        reportCount: g.members.reduce((sum, a) => sum + a.reportCount, 0),
        hasUnknownUsage: folded.hasUnknownUsage,
      };
    }),
  };
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

  /**
   * `basisTotals` is the one block in this fixture that was NOT captured: it
   * was added to the response after the fixture was taken, and folded out of
   * the fixture's own agent totals (see `_provenance.basisTotals`). So it gets
   * checked rather than trusted.
   *
   * Two independent partitions of the same window are folded here — by agent
   * and by day — and both must reproduce the committed block. A hand-edited
   * figure fails, and so does a regeneration that emits an inconsistent one.
   */
  test("the committed basisTotals block is the fold of the fixture's own rows", async () => {
    const series = liveFixture.series;

    expect(series.basisTotals.map((g) => g.basis)).toEqual(series.bases);
    expect(series.basisTotals.reduce((sum, g) => sum + g.reportCount, 0)).toBe(
      series.coverage.reportCount,
    );

    // Beyond one basis the groups partition the rows and no single fold
    // reproduces any one of them, so the equality below is scoped to the
    // single-basis window this fixture actually holds.
    expect(series.bases.length).toBe(1);
    const group = series.basisTotals[0];
    const byAgent = foldUsage(
      series.agents.map((a) => a.usage as ReportedUsage),
    );
    const byDay = foldUsage(
      series.buckets.map((b) => b.usage as ReportedUsage),
    );

    for (const field of TOKEN_FIELDS) {
      expect(
        group.usage[field],
        `${field} must be the fold over agents`,
      ).toEqual(byAgent.usage[field]);
      expect(byDay.usage[field], `${field} must fold the same by day`).toEqual(
        byAgent.usage[field],
      );
    }
    expect(group.usage.freshInputTokens).toEqual(
      byAgent.usage.freshInputTokens,
    );
    expect(group.hasUnknownUsage).toBe(byAgent.hasUnknownUsage);

    // Cost is compared within a tolerance, not exactly: f64 addition is not
    // associative, so folding by agent, by day and by event land on different
    // last bits of the same number.
    expect(group.usage.estimatedCostUsd.value).toBeCloseTo(
      byAgent.usage.estimatedCostUsd.value as number,
      6,
    );
    expect(byDay.usage.estimatedCostUsd.value).toBeCloseTo(
      byAgent.usage.estimatedCostUsd.value as number,
      6,
    );
    expect(group.usage.estimatedCostUsd.incomplete).toBe(
      byAgent.usage.estimatedCostUsd.incomplete,
    );
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

    // The window total, labelled by how its turns were counted. One group
    // here because this window is single-basis; the mixed case is below.
    const basisSection = card.getByTestId("agent-usage-basis-totals");
    await expect(basisSection).toBeVisible();
    const groups = basisSection.getByTestId("agent-usage-basis-total");
    await expect(groups).toHaveCount(liveFixture.series.bases.length);
    const group = liveFixture.series.basisTotals[0];
    const groupRow = basisSection.locator(`[data-basis="${group.basis}"]`);
    // This fixture's turns declared no basis, so the panel must say the basis
    // was not declared rather than implying the counts are missing.
    await expect(groupRow).toContainText("Basis not declared");
    await expect(groupRow).toContainText(
      `${group.reportCount.toLocaleString("en-US")} turns`,
    );
    for (const field of [
      group.usage.inputTokens,
      group.usage.outputTokens,
      group.usage.cacheReadTokens,
    ]) {
      if (field.value !== null)
        await expect(groupRow).toContainText(grouped(field.value));
    }

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

  /**
   * A window whose publishers count a turn differently has no combined figure
   * — NIP-AM forbids summing across bases, so the backend reports the combined
   * series as unknown. The panel must answer that with one real total per
   * basis, never with a blank where a number belongs.
   */
  test("a mixed-basis window shows a labelled total per basis, not a blank", async ({
    page,
  }) => {
    const series = mixedBasisSeries();
    await installMockBridge(
      page,
      {
        agentUsageSeries: series,
        managedAgents: liveFixture.agentNames.map((a) => ({
          pubkey: a.pubkey,
          name: a.name,
        })),
      },
      { seedPreviewFeatures: true },
    );

    const card = await openAgentUsageSettings(page);
    const basisSection = card.getByTestId("agent-usage-basis-totals");
    await expect(basisSection).toBeVisible();
    await expect(
      basisSection.getByTestId("agent-usage-basis-total"),
    ).toHaveCount(2);

    // ...and it must say why there are two, rather than leaving the operator
    // to guess that the panel is broken.
    await expect(card.getByTestId("agent-usage-mixed-bases")).toContainText(
      "no single combined figure",
    );

    const labels: Record<string, string> = {
      per_turn_sum: "Whole turn (all model calls)",
      per_response: "Single model response",
    };
    for (const group of series.basisTotals) {
      const groupRow = basisSection.locator(`[data-basis="${group.basis}"]`);
      await expect(groupRow).toContainText(labels[group.basis]);
      await expect(groupRow).toContainText(
        `${group.reportCount.toLocaleString("en-US")} turns`,
      );
      // Each group carries its own real figure. This is the positive claim the
      // absence check below is anchored to: the subject is on screen with its
      // own number before anything is asserted to be missing.
      const input = group.usage.inputTokens.value;
      expect(
        input,
        "each basis group must have a real input total",
      ).not.toBeNull();
      await expect(groupRow).toContainText(grouped(input as string));
    }

    // The cross-basis sum is the number the whole rule exists to suppress. It
    // must appear nowhere, in any group or in the combined row.
    const crossBasisSum = series.basisTotals.reduce(
      (sum, g) => sum + BigInt(g.usage.inputTokens.value as string),
      0n,
    );
    await expect(card).not.toContainText(grouped(String(crossBasisSum)));

    await settleAnimations(card);
    await basisSection.scrollIntoViewIfNeeded();
    await settleAnimations(card);
    await basisSection.screenshot({
      path: `${SHOTS}/03-mixed-basis-totals.png`,
    });
  });
});
