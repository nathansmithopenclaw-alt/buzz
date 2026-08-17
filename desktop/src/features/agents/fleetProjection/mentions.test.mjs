import assert from "node:assert/strict";
import test from "node:test";

import { mentionableProjectionAgents } from "./mentions.ts";

const VIEWER = "1".repeat(64);
const TEAM_OWNER = "2".repeat(64);
const OWN_AGENT = "3".repeat(64);
const TEAM_AGENT = "4".repeat(64);

function agent(overrides) {
  return {
    ownerPubkey: TEAM_OWNER,
    agentPubkey: TEAM_AGENT,
    name: "Team agent",
    personaId: null,
    personaDisplayName: null,
    personaAvatarUrl: null,
    respondTo: "anyone",
    respondToAllowlist: [],
    ...overrides,
  };
}

test("own projections are eligible without channel membership", () => {
  const agents = mentionableProjectionAgents({
    owners: [
      {
        ownerPubkey: VIEWER,
        agents: [agent({ ownerPubkey: VIEWER, agentPubkey: OWN_AGENT })],
        hosts: [],
      },
    ],
    channelType: "dm",
    currentPubkey: VIEWER,
    memberPubkeys: new Set(),
  });
  assert.deepEqual(
    agents.map((candidate) => candidate.agentPubkey),
    [OWN_AGENT],
  );
});

test("team projections keep channel membership and respond-to eligibility", () => {
  const owners = [
    {
      ownerPubkey: TEAM_OWNER,
      agents: [
        agent({}),
        agent({
          agentPubkey: "5".repeat(64),
          respondTo: "allowlist",
          respondToAllowlist: [VIEWER],
        }),
        agent({
          agentPubkey: "6".repeat(64),
          respondTo: "allowlist",
          respondToAllowlist: [],
        }),
      ],
      hosts: [],
    },
  ];
  const agents = mentionableProjectionAgents({
    owners,
    channelType: "stream",
    currentPubkey: VIEWER,
    memberPubkeys: new Set([TEAM_AGENT, "5".repeat(64), "6".repeat(64)]),
  });
  assert.deepEqual(
    agents.map((candidate) => candidate.agentPubkey),
    [TEAM_AGENT, "5".repeat(64)],
  );
});
