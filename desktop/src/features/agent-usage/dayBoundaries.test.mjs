import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { areBoundariesValid, lastNDaysBoundaries } from "./dayBoundaries.ts";

const DAY = 86_400;

describe("lastNDaysBoundaries", () => {
  it("returns days + 1 boundaries", () => {
    assert.equal(lastNDaysBoundaries(7).length, 8);
    assert.equal(lastNDaysBoundaries(30).length, 31);
    assert.equal(lastNDaysBoundaries(1).length, 2);
  });

  it("ends after today so the partial current day is the last bucket", () => {
    const now = new Date();
    const boundaries = lastNDaysBoundaries(7, now);
    const end = boundaries[boundaries.length - 1] * 1000;
    assert.ok(end > now.getTime(), "the window must extend past now");
    assert.ok(
      end - now.getTime() <= 25 * 3600 * 1000,
      "but no further than tomorrow's local midnight",
    );
  });

  it("puts every boundary at local midnight", () => {
    for (const t of lastNDaysBoundaries(30)) {
      const d = new Date(t * 1000);
      assert.equal(d.getHours(), 0, `${d} is not local midnight`);
      assert.equal(d.getMinutes(), 0);
      assert.equal(d.getSeconds(), 0);
    }
  });

  it("is strictly increasing and never exceeds the 48h validator bound", () => {
    const boundaries = lastNDaysBoundaries(30);
    for (let i = 0; i < boundaries.length - 1; i += 1) {
      const interval = boundaries[i + 1] - boundaries[i];
      assert.ok(interval > 0, `interval ${i} must be positive`);
      assert.ok(interval <= 48 * 3600, `interval ${i} exceeds 48h`);
    }
  });

  it("spans a DST transition without drifting off local midnight", () => {
    // US DST forward is the second Sunday in March: 2026-03-08. A range that
    // crosses it contains one 23-hour civil day. A fixed 86_400 stride would
    // hold the interval at exactly a day and slide every later boundary an
    // hour off local midnight — the bug this helper exists to prevent.
    const boundaries = lastNDaysBoundaries(5, new Date(2026, 2, 10, 12, 0, 0));
    for (const t of boundaries) {
      assert.equal(new Date(t * 1000).getHours(), 0);
    }
    const intervals = [];
    for (let i = 0; i < boundaries.length - 1; i += 1) {
      intervals.push(boundaries[i + 1] - boundaries[i]);
    }
    const offNominal = intervals.filter((i) => i !== DAY);
    // In a DST-observing zone exactly one interval is short; in a zone with no
    // DST every interval is nominal. Both are correct — what must never happen
    // is an interval that is neither.
    for (const interval of offNominal) {
      assert.ok(
        interval === DAY - 3600 || interval === DAY + 3600,
        `unexpected interval ${interval}`,
      );
    }
  });

  it("clamps to the backend's accepted boundary range", () => {
    // The caller cannot construct a request the Rust validator rejects.
    assert.equal(lastNDaysBoundaries(0).length, 2);
    assert.equal(lastNDaysBoundaries(-5).length, 2);
    assert.equal(lastNDaysBoundaries(10_000).length, 367);
  });
});

describe("areBoundariesValid", () => {
  it("accepts what lastNDaysBoundaries produces", () => {
    for (const days of [1, 7, 30, 366]) {
      assert.equal(
        areBoundariesValid(lastNDaysBoundaries(days)),
        true,
        `${days}`,
      );
    }
  });

  it("rejects a list that is too short, too long, or not increasing", () => {
    assert.equal(areBoundariesValid([1]), false);
    assert.equal(
      areBoundariesValid(new Array(368).fill(0).map((_, i) => i * DAY)),
      false,
    );
    assert.equal(areBoundariesValid([DAY * 2, DAY]), false);
    assert.equal(areBoundariesValid([DAY, DAY]), false);
  });

  it("rejects an interval wider than the validator's 48h bound", () => {
    assert.equal(areBoundariesValid([0, 49 * 3600]), false);
    assert.equal(areBoundariesValid([0, 48 * 3600]), true);
  });
});
