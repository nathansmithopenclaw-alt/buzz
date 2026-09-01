import assert from "node:assert/strict";
import test from "node:test";

import { prepareAssigneeDirectoryCandidates } from "./assigneeCandidates.ts";

function candidate(pubkey, displayName, overrides = {}) {
  return {
    avatarUrl: null,
    displayName,
    isAgent: true,
    nip05Handle: null,
    ownerPubkey: null,
    pubkey,
    ...overrides,
  };
}

test("assignee directory keeps the live identity and collapses stale duplicate names", () => {
  const current = candidate("c".repeat(64), "Honey");
  const staleA = candidate("a".repeat(64), "Honey");
  const staleB = candidate("b".repeat(64), "Honey");

  assert.deepEqual(
    prepareAssigneeDirectoryCandidates({
      candidates: [staleA, staleB, current],
      preferredPubkeys: new Set([current.pubkey]),
    }),
    [current],
  );
});

test("assignee directory preserves distinct live people who share a name", () => {
  const first = candidate("a".repeat(64), "Sam", { isAgent: false });
  const second = candidate("b".repeat(64), "Sam", { isAgent: false });

  assert.deepEqual(
    prepareAssigneeDirectoryCandidates({
      candidates: [first, second],
      preferredPubkeys: new Set([first.pubkey, second.pubkey]),
    }),
    [first, second],
  );
});

test("assignee directory suppresses blank profiles and collapses stale-only groups", () => {
  const staleA = candidate("a".repeat(64), "Retired Agent");
  const staleB = candidate("b".repeat(64), "Retired Agent");
  const blank = candidate("c".repeat(64), "  ");

  assert.deepEqual(
    prepareAssigneeDirectoryCandidates({
      candidates: [staleA, staleB, blank],
      preferredPubkeys: new Set(),
    }),
    [staleA],
  );
});
