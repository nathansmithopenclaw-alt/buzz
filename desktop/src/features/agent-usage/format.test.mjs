import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  cacheHitRate,
  costDisplay,
  formatTokenCount,
  isTeardownCause,
  peakContextSummary,
  provenanceRatio,
  sessionCauseLabel,
  totalSessions,
  usageDisplay,
} from "./format.ts";

const known = (value) => ({ value, incomplete: false });
const partial = (value) => ({ value, incomplete: true });
const absent = { value: null, incomplete: false };

describe("formatTokenCount", () => {
  it("keeps full precision past 2^53", () => {
    // Counters arrive as decimal strings precisely because a u64 does not
    // survive JSON's number type. Parsing through Number would round this.
    assert.equal(
      formatTokenCount("18446744073709551615"),
      "18,446,744,073,709,551,615",
    );
  });

  it("returns null for an absent value rather than zero", () => {
    assert.equal(formatTokenCount(null), null);
  });

  it("returns null for a malformed value rather than throwing", () => {
    assert.equal(formatTokenCount("not-a-number"), null);
  });

  it("preserves a genuine zero", () => {
    // Some(0) means the provider confirmed zero; it must not be flattened
    // into the unknown case.
    assert.equal(formatTokenCount("0"), "0");
  });
});

describe("usageDisplay", () => {
  it("distinguishes unknown from zero", () => {
    assert.deepEqual(usageDisplay(absent), { text: null, partial: false });
    assert.deepEqual(usageDisplay(known("0")), { text: "0", partial: false });
  });

  it("marks a reported lower bound as partial", () => {
    assert.deepEqual(usageDisplay(partial("500")), {
      text: "500",
      partial: true,
    });
  });
});

describe("costDisplay", () => {
  it("uses more precision for sub-dollar estimates", () => {
    assert.equal(costDisplay(known(0.0123)).text, "$0.0123");
    assert.equal(costDisplay(known(12.5)).text, "$12.50");
  });

  it("returns null text when no cost was reported", () => {
    assert.equal(costDisplay({ value: null, incomplete: true }).text, null);
  });
});

describe("cacheHitRate", () => {
  it("computes a percentage from two exact figures", () => {
    assert.equal(cacheHitRate(known("1000"), known("975")), "97.5%");
  });

  it("scales before dividing so small ratios are not floored to zero", () => {
    assert.equal(cacheHitRate(known("1000000"), known("1")), "0.0%");
    assert.equal(cacheHitRate(known("10000"), known("1")), "0.0%");
    assert.equal(cacheHitRate(known("1000"), known("5")), "0.5%");
  });

  it("refuses to derive a ratio from a lower bound", () => {
    // A ratio built from partial data reads as precise and is not.
    assert.equal(cacheHitRate(partial("1000"), known("975")), null);
    assert.equal(cacheHitRate(known("1000"), partial("975")), null);
  });

  it("returns null when either figure is unknown or input is zero", () => {
    assert.equal(cacheHitRate(absent, known("975")), null);
    assert.equal(cacheHitRate(known("1000"), absent), null);
    assert.equal(cacheHitRate(known("0"), known("0")), null);
  });
});

describe("sessionCauseLabel", () => {
  it("labels the known causes", () => {
    assert.equal(sessionCauseLabel("rotation_turn_limit"), "Turn-limit rotation");
    assert.equal(sessionCauseLabel("owner_interrupt"), "Owner interrupted");
  });

  it("renders an unrecognized cause verbatim, not as unknown", () => {
    // NIP-AM lets publishers add causes. Swallowing the wire value would hide
    // a real teardown reason behind a shrug.
    assert.equal(sessionCauseLabel("future_cause"), "future_cause");
  });

  it("says a missing cause was not reported", () => {
    assert.equal(sessionCauseLabel(null), "Not reported");
  });
});

describe("isTeardownCause", () => {
  it("excludes the two no-predecessor causes", () => {
    assert.equal(isTeardownCause("worker_spawn"), false);
    assert.equal(isTeardownCause("scope_first_turn"), false);
    assert.equal(isTeardownCause(null), false);
  });

  it("includes every cause that replaced a live session", () => {
    for (const cause of [
      "rotation_turn_limit",
      "provider_limit",
      "owner_rotate",
      "owner_interrupt",
      "model_switch",
      "membership_change",
      "adapter_failure",
      "agent_exited",
      "turn_timeout",
      "error",
    ]) {
      assert.equal(isTeardownCause(cause), true, cause);
    }
  });
});

describe("totalSessions", () => {
  it("sums every bucket", () => {
    assert.equal(
      totalSessions([
        { cause: "worker_spawn", sessions: 3 },
        { cause: null, sessions: 2 },
      ]),
      5,
    );
  });
});

describe("provenanceRatio", () => {
  it("returns null when nothing reported the field", () => {
    // 0 of 0 is not evidence of zero retries — it is the absence of evidence,
    // and the panel must say so rather than render "0".
    assert.equal(provenanceRatio(0, 0), null);
  });

  it("reports the count against its denominator", () => {
    assert.equal(provenanceRatio(0, 28), "0 of 28");
    assert.equal(provenanceRatio(4, 28), "4 of 28");
  });
});

describe("peakContextSummary", () => {
  it("pairs peak occupancy with the window when both are known", () => {
    assert.equal(
      peakContextSummary({
        peakContextTokens: "145213",
        contextWindow: "200000",
      }),
      "145,213 / 200,000",
    );
  });

  it("shows peak alone when no window was reported", () => {
    assert.equal(
      peakContextSummary({
        peakContextTokens: "145213",
        contextWindow: null,
      }),
      "145,213",
    );
  });

  it("returns null when no context was reported at all", () => {
    assert.equal(
      peakContextSummary({ peakContextTokens: null, contextWindow: "200000" }),
      null,
    );
  });
});
