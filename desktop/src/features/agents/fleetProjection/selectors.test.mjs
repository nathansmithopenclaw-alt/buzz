import assert from "node:assert/strict";
import test from "node:test";

import {
  fleetAgentLiveness,
  fleetAgentHost,
  fleetAgentObserverStatus,
  fleetHostIsFresh,
  presentFleetProjectionAgent,
} from "./selectors.ts";

const OWNER = "1".repeat(64);
const AGENT = "2".repeat(64);

const agent = {
  ownerPubkey: OWNER,
  agentPubkey: AGENT,
  name: "Projection fallback",
  personaId: "reviewer",
  personaDisplayName: "Persona name",
  personaAvatarUrl: "https://example.test/persona.png",
  personaRuntime: "codex",
  personaModel: "gpt-5",
  respondTo: "owner-only",
  respondToAllowlist: [],
};

test("presentation follows persona, kind-0, kind-10100, then 30177 fallback order", () => {
  assert.equal(
    presentFleetProjectionAgent(
      agent,
      { displayName: "Kind zero", avatarUrl: "kind-zero.png" },
      { name: "Agent profile" },
    ).displayName,
    "Persona name",
  );
  assert.equal(
    presentFleetProjectionAgent(
      { ...agent, personaDisplayName: null, personaAvatarUrl: null },
      { displayName: "Kind zero", avatarUrl: "kind-zero.png" },
      { name: "Agent profile" },
    ).displayName,
    "Kind zero",
  );
  assert.equal(
    presentFleetProjectionAgent(
      { ...agent, personaDisplayName: null, personaAvatarUrl: null },
      null,
      { name: "Agent profile" },
    ).displayName,
    "Agent profile",
  );
  assert.equal(
    presentFleetProjectionAgent(
      { ...agent, personaDisplayName: null, personaAvatarUrl: null },
      null,
      null,
    ).displayName,
    "Projection fallback",
  );
});

test("90 seconds is fresh; 91 seconds greys agent state as unknown", () => {
  const owner = {
    ownerPubkey: OWNER,
    agents: [agent],
    personas: [],
    hosts: [
      {
        ownerPubkey: OWNER,
        hostId: "018f42c2-9bc2-7dc1-a41e-2f77769f0142",
        name: "Build host",
        platform: "macos",
        declaredAt: 1,
        status: {
          seq: 1,
          timestamp: 1_000,
          agents: [{ pubkey: AGENT, state: "running", since: 950 }],
        },
      },
    ],
  };
  assert.equal(fleetHostIsFresh(owner.hosts[0], 1_090), true);
  assert.deepEqual(fleetAgentLiveness(owner, AGENT, 1_090), {
    state: "running",
    since: 950,
    stale: false,
  });
  assert.equal(fleetHostIsFresh(owner.hosts[0], 1_091), false);
  assert.deepEqual(fleetAgentLiveness(owner, AGENT, 1_091), {
    state: "unknown",
    since: null,
    stale: true,
  });
});

test("Observer vocabulary derives live, quiet, and host-unreachable without inventing a host", () => {
  const host = {
    ownerPubkey: OWNER,
    hostId: "018f42c2-9bc2-7dc1-a41e-2f77769f0142",
    name: "Build host",
    platform: "macos",
    declaredAt: 1,
    status: {
      seq: 1,
      timestamp: 1_000,
      agents: [{ pubkey: AGENT, state: "running", since: 950 }],
    },
  };
  const owner = {
    ownerPubkey: OWNER,
    agents: [agent],
    personas: [],
    hosts: [host],
  };
  assert.equal(fleetAgentHost(owner, AGENT), host);
  assert.deepEqual(fleetAgentObserverStatus(owner, AGENT, 1_090), {
    state: "live",
    lastSeen: 1_000,
    host,
  });
  host.status.agents[0].state = "stopped";
  assert.deepEqual(fleetAgentObserverStatus(owner, AGENT, 1_090), {
    state: "quiet",
    lastSeen: 1_000,
    host,
  });
  assert.deepEqual(fleetAgentObserverStatus(owner, AGENT, 1_091), {
    state: "host-unreachable",
    lastSeen: 1_000,
    host,
  });
  assert.deepEqual(
    fleetAgentObserverStatus({ ...owner, hosts: [] }, AGENT, 1_090),
    { state: "quiet", lastSeen: null, host: null },
  );
});
