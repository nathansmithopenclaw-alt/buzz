import assert from "node:assert/strict";
import test from "node:test";

import {
  fleetProjectionHeadBackfillFilter,
  membershipScopedFleetOwners,
} from "./sync.ts";
import { createFleetProjectionCache } from "./store.ts";

const VIEWER = "1".repeat(64);
const SHARED = "2".repeat(64);
const ARCHIVED = "3".repeat(64);
const OUTSIDER = "4".repeat(64);

function channel(overrides) {
  return {
    id: "channel",
    name: "Channel",
    channelType: "stream",
    visibility: "private",
    description: "",
    topic: null,
    purpose: null,
    memberCount: 0,
    memberPubkeys: [],
    lastMessageAt: null,
    archivedAt: null,
    participants: [],
    participantPubkeys: [],
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
    ...overrides,
  };
}

test("team owner enumeration is limited to active channels the viewer shares", () => {
  const owners = membershipScopedFleetOwners(VIEWER, [
    channel({ memberPubkeys: [SHARED] }),
    channel({
      id: "archived",
      archivedAt: "2026-01-01",
      memberPubkeys: [ARCHIVED],
    }),
    channel({ id: "not-member", isMember: false, memberPubkeys: [OUTSIDER] }),
  ]);
  assert.deepEqual(owners, [VIEWER, SHARED]);
});

test("head backfill survives tombstone volume above the relay response cap", () => {
  const agentPubkey = "5".repeat(64);
  const oldHead = {
    id: "a".repeat(64),
    pubkey: VIEWER,
    created_at: 100,
    kind: 30177,
    tags: [["d", agentPubkey]],
    content: JSON.stringify({
      name: "Long-lived agent",
      persona_id: null,
      respond_to: "owner-only",
      respond_to_allowlist: [],
    }),
    sig: "f".repeat(128),
  };
  const newerTombstones = Array.from({ length: 2_500 }, (_, index) => ({
    id: index.toString(16).padStart(64, "0"),
    pubkey: VIEWER,
    created_at: 101 + index,
    kind: 5,
    tags: [["a", `30177:${VIEWER}:${index.toString(16).padStart(64, "0")}`]],
    content: "",
    sig: "e".repeat(128),
  }));
  const relayHistory = [oldHead, ...newerTombstones].sort(
    (left, right) => right.created_at - left.created_at,
  );
  const relayCap = 1_000;
  const query = (filter) =>
    relayHistory
      .filter(
        (event) =>
          filter.kinds.includes(event.kind) &&
          (filter.authors ?? []).includes(event.pubkey),
      )
      .slice(0, Math.min(filter.limit, relayCap));

  const mixedPage = query({
    kinds: [5, 30175, 30177, 30180, 30181],
    authors: [VIEWER],
    limit: 2_000,
  });
  assert.equal(
    mixedPage.some((event) => event.id === oldHead.id),
    false,
  );

  const cache = createFleetProjectionCache();
  cache.reset([VIEWER], true);
  cache.ingestMany(query(fleetProjectionHeadBackfillFilter([VIEWER])));
  cache.finishLoading();

  assert.equal(
    cache.getSnapshot().owners[0].agents[0].agentPubkey,
    agentPubkey,
  );
  assert.equal(
    cache.getSnapshot().owners[0].agents[0].name,
    "Long-lived agent",
  );
});
