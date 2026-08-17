import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveFleetAgentIdentity,
  resolveFleetAgentRuntimeLabel,
} from "./agentFleetIdentity.ts";

const agent = {
  avatarUrl: "pinned.png",
  name: "Pinned instance",
  pubkey: "0123456789abcdef",
};
const persona = { avatarUrl: "definition.png", displayName: "Definition" };

test("fleet identity prefers a real kind-0 profile", () => {
  assert.deepEqual(
    resolveFleetAgentIdentity({
      agent,
      persona,
      profile: {
        avatarUrl: "profile.png",
        displayName: "Profile",
        hasProfileEvent: true,
      },
    }),
    { avatarUrl: "profile.png", displayName: "Profile" },
  );
});

test("fleet identity falls back to the definition before pinned instance identity", () => {
  assert.deepEqual(
    resolveFleetAgentIdentity({
      agent,
      persona,
      profile: {
        avatarUrl: null,
        displayName: null,
        hasProfileEvent: false,
      },
    }),
    { avatarUrl: "definition.png", displayName: "Definition" },
  );
});

test("fleet identity retains pinned identity for an orphaned instance", () => {
  assert.deepEqual(resolveFleetAgentIdentity({ agent }), {
    avatarUrl: "pinned.png",
    displayName: "Pinned instance",
  });
});

test("fleet identity degrades to compact pubkey text only after other identity sources", () => {
  assert.deepEqual(
    resolveFleetAgentIdentity({
      agent: {
        avatarUrl: null,
        name: " ",
        pubkey: "0123456789abcdef",
      },
    }),
    { avatarUrl: null, displayName: "012345…abcdef" },
  );
});

test("fleet runtime prefers an explicit instance runtime", () => {
  assert.equal(
    resolveFleetAgentRuntimeLabel({
      agent: { runtime: "goose" },
      persona: { runtime: "codex" },
    }),
    "goose",
  );
});

test("fleet runtime falls back from the instance to its definition", () => {
  assert.equal(
    resolveFleetAgentRuntimeLabel({
      agent: { runtime: null },
      persona: { runtime: "codex" },
    }),
    "codex",
  );
});

test("fleet runtime names the inherited default honestly", () => {
  assert.equal(
    resolveFleetAgentRuntimeLabel({ agent: { runtime: null } }),
    "Default runtime",
  );
});
