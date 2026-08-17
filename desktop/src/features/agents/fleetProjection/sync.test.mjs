import assert from "node:assert/strict";
import test from "node:test";

import { membershipScopedFleetOwners } from "./sync.ts";

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
