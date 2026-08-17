import assert from "node:assert/strict";
import test from "node:test";

import { createFleetProjectionCache } from "./store.ts";

const OWNER = "1".repeat(64);
const AGENT = "2".repeat(64);
const PERSONA = "reviewer";
const HOST = "018f42c2-9bc2-7dc1-a41e-2f77769f0142";

function event({ kind, content, tags, createdAt = 100, id = "a".repeat(64) }) {
  return {
    id,
    pubkey: OWNER,
    created_at: createdAt,
    kind,
    tags,
    content: typeof content === "string" ? content : JSON.stringify(content),
    sig: "f".repeat(128),
  };
}

function agentEvent(createdAt = 100) {
  return event({
    kind: 30177,
    createdAt,
    tags: [["d", AGENT]],
    content: {
      name: "Fallback Agent",
      persona_id: PERSONA,
      respond_to: "owner-only",
      respond_to_allowlist: [],
      parallelism: 1,
      system_prompt: "public but deliberately not retained by this cache",
      private_key_nsec: "must-not-survive",
      auth_tag: "must-not-survive",
      env_vars: { SECRET: "must-not-survive" },
      backend: { type: "must-not-survive" },
    },
  });
}

test("projection cache joins definitions and host liveness without runnable fields", () => {
  const cache = createFleetProjectionCache();
  cache.reset([OWNER], true);
  cache.ingestMany([
    agentEvent(),
    event({
      kind: 30175,
      tags: [["d", PERSONA]],
      content: {
        display_name: "Review Agent",
        avatar_url: "https://example.test/reviewer.png",
      },
    }),
    event({
      kind: 30180,
      tags: [["d", HOST]],
      content: {
        v: 1,
        name: "Build host",
        platform: "macos",
        created_at: 90,
      },
    }),
    event({
      kind: 30181,
      tags: [["d", HOST]],
      content: {
        v: 1,
        seq: 4,
        ts: 100,
        agents: [{ pubkey: AGENT, state: "running", since: 95 }],
      },
    }),
  ]);
  cache.finishLoading();

  const snapshot = cache.getSnapshot();
  assert.equal(snapshot.loading, false);
  assert.equal(snapshot.owners[0].agents[0].personaDisplayName, "Review Agent");
  assert.equal(snapshot.owners[0].hosts[0].status.agents[0].state, "running");
  const serialized = JSON.stringify(snapshot);
  for (const forbidden of [
    "private_key_nsec",
    "auth_tag",
    "env_vars",
    "backend",
    "system_prompt",
    "must-not-survive",
  ]) {
    assert.equal(serialized.includes(forbidden), false, forbidden);
  }
});

test("coordinate tombstones remove heads and only a later event recreates them", () => {
  const cache = createFleetProjectionCache();
  cache.reset([OWNER], false);
  cache.ingest(agentEvent(100));
  cache.ingest(
    event({
      kind: 5,
      createdAt: 110,
      id: "b".repeat(64),
      tags: [["a", `30177:${OWNER}:${AGENT}`]],
      content: "",
    }),
  );
  assert.equal(cache.getSnapshot().owners.length, 0);

  cache.ingest(agentEvent(105));
  assert.equal(cache.getSnapshot().owners.length, 0);
  cache.ingest(agentEvent(111));
  assert.equal(cache.getSnapshot().owners[0].agents.length, 1);
});

test("events from authors outside the membership scope are ignored", () => {
  const cache = createFleetProjectionCache();
  cache.reset(["3".repeat(64)], false);
  cache.ingest(agentEvent());
  assert.deepEqual(cache.getSnapshot().owners, []);
});
