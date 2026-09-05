import assert from "node:assert/strict";
import { test } from "node:test";
import {
  subscriptionAccountExpiry,
  subscriptionAccountNeedsRenewal,
} from "./subscriptionAccountExpiry.ts";

test("token expiry uses seconds, rounds partial days up and expires at the exact boundary", () => {
  const minted = 1800000000;
  const account = {
    kind: "token",
    minted_at: minted,
    expires_at: minted + 365 * 86400,
  };
  for (const [now, label, expired] of [
    [minted, "Expires in 365 days", false],
    [account.expires_at - 86401, "Expires in 2 days", false],
    [account.expires_at - 86400, "Expires in 1 day", false],
    [account.expires_at - 0.001, "Expires in 1 day", false],
    [account.expires_at, "Expired", true],
    [account.expires_at + 1, "Expired", true],
  ])
    assert.deepEqual(subscriptionAccountExpiry(account, now), {
      label,
      expired,
    });
});

test("legacy login mocks have no expiry; missing token expiry stays visibly unknown", () => {
  assert.equal(subscriptionAccountExpiry({}), null);
  assert.equal(
    subscriptionAccountExpiry({ kind: "login", expires_at: 1 }),
    null,
  );
  assert.deepEqual(subscriptionAccountExpiry({ kind: "token" }), {
    label: "Expiry unavailable",
    expired: false,
  });
});

test("persistent renewal notice starts at exactly 30 days and remains after expiry", () => {
  const now = 1800000000;
  for (const [secondsRemaining, expected] of [
    [30 * 86400 + 1, false],
    [30 * 86400, true],
    [1, true],
    [0, true],
    [-86400, true],
  ])
    assert.equal(
      subscriptionAccountNeedsRenewal(
        { kind: "token", expires_at: now + secondsRemaining },
        now,
      ),
      expected,
    );
  assert.equal(
    subscriptionAccountNeedsRenewal({ kind: "token", expires_at: null }, now),
    false,
  );
  assert.equal(
    subscriptionAccountNeedsRenewal({ kind: "login", expires_at: now }, now),
    false,
  );
});
