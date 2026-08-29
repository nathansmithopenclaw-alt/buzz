import assert from "node:assert/strict";
import test from "node:test";

import {
  createDispatchHistoryClient,
  DispatchHistoryGapError,
  isDispatchHistoryGap,
  isRateLimitRejection,
} from "./history.ts";

function event(id, createdAt) {
  return {
    id: id.toString(16).padStart(64, "0"),
    pubkey: "a".repeat(64),
    created_at: createdAt,
    kind: 40020,
    tags: [],
    content: "",
    sig: "b".repeat(128),
  };
}

function client(overrides) {
  return createDispatchHistoryClient({
    waitForRateLimit: async () => {},
    isRateLimited: () => false,
    activateRateLimit: () => {},
    requestIntervalMs: 0,
    ...overrides,
  });
}

test("rate-limit rejection classification stays typed and narrow", () => {
  assert.equal(
    isRateLimitRejection(
      new Error("rate-limited: quota exceeded; retry in 1s"),
    ),
    true,
  );
  assert.equal(
    isRateLimitRejection(new Error("relay rate-limited: retry in 1s")),
    true,
  );
  assert.equal(isRateLimitRejection(new Error("database unavailable")), false);
});

test("exhaustive pagination gates every page and timestamp boundary", async () => {
  const firstPage = Array.from({ length: 500 }, (_, index) =>
    event(index + 1, 200 - Math.floor(index / 250)),
  );
  const pages = [firstPage, [event(501, 199)], [event(502, 198)]];
  const filters = [];
  let gateWaits = 0;
  const history = client({
    waitForRateLimit: async () => {
      gateWaits += 1;
    },
    fetchEvents: async (filter) => {
      filters.push(filter);
      return pages.shift() ?? [];
    },
  });

  const events = await history.fetchExhaustively({
    kinds: [40020, 40021],
    "#h": ["workspace"],
  });

  assert.equal(gateWaits, 3);
  assert.equal(filters.length, 3);
  assert.deepEqual(
    { since: filters[1].since, until: filters[1].until },
    { since: 199, until: 199 },
  );
  assert.equal(filters[2].until, 198);
  assert.equal(events.length, 502);
});

test("a saturated timestamp boundary is a typed cursor gap", async () => {
  const dense = Array.from({ length: 500 }, (_, index) =>
    event(index + 1, 200),
  );
  const history = client({ fetchEvents: async () => dense });

  await assert.rejects(
    history.fetchExhaustively({ kinds: [40020], since: 100 }),
    (error) => {
      assert.equal(error instanceof DispatchHistoryGapError, true);
      assert.equal(isDispatchHistoryGap(error), true);
      return true;
    },
  );
});

test("cursor pagination stops before constructing an inverted since/until range", async () => {
  const firstPage = Array.from({ length: 500 }, (_, index) =>
    event(index + 1, index === 499 ? 100 : 200),
  );
  const pages = [firstPage, [event(501, 100)]];
  const filters = [];
  const history = client({
    fetchEvents: async (filter) => {
      filters.push(filter);
      return pages.shift() ?? [];
    },
  });

  const events = await history.fetchExhaustively({
    kinds: [40020],
    since: 100,
  });

  assert.equal(events.length, 501);
  assert.equal(filters.length, 2);
  assert.deepEqual(
    { since: filters[1].since, until: filters[1].until },
    { since: 100, until: 100 },
  );
});

test("serial queue paces concurrent cold-load streams", async () => {
  let now = 0;
  const starts = [];
  let inFlight = 0;
  let peakInFlight = 0;
  const history = client({
    requestIntervalMs: 150,
    now: () => now,
    sleep: async (milliseconds) => {
      now += milliseconds;
    },
    fetchEvents: async () => {
      starts.push(now);
      inFlight += 1;
      peakInFlight = Math.max(peakInFlight, inFlight);
      await Promise.resolve();
      inFlight -= 1;
      return [];
    },
  });

  await Promise.all([
    history.fetchExhaustively({ kinds: [40020] }),
    history.fetchExhaustively({ kinds: [40022] }),
    history.fetchExhaustively({ kinds: [30621] }),
    history.fetchExhaustively({ kinds: [9] }),
  ]);

  assert.equal(peakInFlight, 1);
  assert.deepEqual(starts, [0, 150, 300, 450]);
});

test("a multi-filter batch spends one queue slot, not one per filter", async () => {
  let now = 0;
  const starts = [];
  const history = client({
    requestIntervalMs: 150,
    now: () => now,
    sleep: async (milliseconds) => {
      now += milliseconds;
    },
    fetchEventsMulti: async () => {
      starts.push(now);
      return [];
    },
  });

  await Promise.all([
    history.fetchMulti([{ kinds: [40020], "#h": ["a"], limit: 1 }]),
    history.fetchMulti([
      { kinds: [40020], "#h": ["b"], limit: 1 },
      { kinds: [40020], "#h": ["c"], limit: 1 },
    ]),
  ]);

  // Two batches carrying three filters pay two pacing gaps, not three.
  assert.deepEqual(starts, [0, 150]);
});

test("an empty batch issues no request", async () => {
  let calls = 0;
  const history = client({
    fetchEventsMulti: async () => {
      calls += 1;
      return [];
    },
  });

  assert.deepEqual(await history.fetchMulti([]), []);
  assert.equal(calls, 0);
});

test("a rate-limited multi-filter batch retries behind the gate", async () => {
  let attempts = 0;
  let gateActive = false;
  const history = client({
    isRateLimited: () => gateActive,
    waitForRateLimit: async () => {
      gateActive = false;
    },
    activateRateLimit: () => {
      gateActive = true;
    },
    fetchEventsMulti: async () => {
      attempts += 1;
      if (attempts === 1) {
        throw new Error("rate-limited: quota exceeded; retry in 1s");
      }
      return [event(1, 100)];
    },
  });

  const events = await history.fetchMulti([
    { kinds: [40020], "#h": ["a"], limit: 1 },
  ]);

  assert.equal(events.length, 1);
  assert.equal(attempts, 2);
});

test("rate-limited page waits behind the gate and then succeeds", async () => {
  let attempts = 0;
  let gateActive = false;
  let gateWaits = 0;
  const retryStates = [];
  const activated = [];
  const history = client({
    isRateLimited: () => gateActive,
    waitForRateLimit: async () => {
      gateWaits += 1;
      gateActive = false;
    },
    activateRateLimit: (message) => {
      activated.push(message);
      gateActive = true;
    },
    onRetryingChange: (retrying) => retryStates.push(retrying),
    fetchEvents: async () => {
      attempts += 1;
      if (attempts === 1) {
        throw new Error("rate-limited: quota exceeded; retry in 1s");
      }
      return [event(1, 100)];
    },
  });

  const events = await history.fetchExhaustively({ kinds: [40020] });

  assert.equal(events.length, 1);
  assert.equal(attempts, 2);
  assert.equal(gateWaits, 2);
  assert.equal(activated.length, 1);
  assert.deepEqual(retryStates, [true, false]);
});

test("rate-limited page retry is bounded", async () => {
  let attempts = 0;
  let gateActive = false;
  const retryStates = [];
  const history = client({
    maxRateLimitRetries: 2,
    isRateLimited: () => gateActive,
    waitForRateLimit: async () => {
      gateActive = false;
    },
    activateRateLimit: () => {
      gateActive = true;
    },
    onRetryingChange: (retrying) => retryStates.push(retrying),
    fetchEvents: async () => {
      attempts += 1;
      throw new Error("rate-limited: quota exceeded; retry in 1s");
    },
  });

  await assert.rejects(
    history.fetchExhaustively({ kinds: [40020] }),
    /rate-limited: quota exceeded/,
  );
  assert.equal(attempts, 3);
  assert.deepEqual(retryStates, [true, false]);
});
