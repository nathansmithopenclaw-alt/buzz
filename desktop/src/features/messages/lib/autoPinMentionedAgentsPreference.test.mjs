import assert from "node:assert/strict";
import test from "node:test";

const values = new Map();
globalThis.localStorage = {
  getItem: (key) => values.get(key) ?? null,
  setItem: (key, value) => values.set(key, String(value)),
};

const preference = await import("./autoPinMentionedAgentsPreference.ts");
const audienceStore = await import("./persistentAgentAudience.ts");
const agentA = "a".repeat(64);
const agentB = "b".repeat(64);
const scope = `${"1".repeat(64)}:channel-a:channel`;

test("defaults missing and invalid values to one-time agent mentions", () => {
  assert.equal(preference.parseKeepMentionedAgentsPinned(null), false);
  assert.equal(preference.parseKeepMentionedAgentsPinned("invalid"), false);
  assert.equal(preference.parseKeepMentionedAgentsPinned("true"), true);
  assert.equal(preference.parseKeepMentionedAgentsPinned("false"), false);
});

test("persists changes to the post-mention pinning preference", () => {
  preference.setKeepMentionedAgentsPinned(true);
  assert.equal(preference.getKeepMentionedAgentsPinned(), true);
  assert.equal(
    values.get(preference.KEEP_MENTIONED_AGENTS_PINNED_STORAGE_KEY),
    "true",
  );

  preference.setKeepMentionedAgentsPinned(false);
  assert.equal(preference.getKeepMentionedAgentsPinned(), false);
  assert.equal(
    values.get(preference.KEEP_MENTIONED_AGENTS_PINNED_STORAGE_KEY),
    "false",
  );
});

test("turning off automatic mentions drops automatic audiences but keeps explicit locks", () => {
  audienceStore.resetPersistentAgentAudienceStore();
  preference.setKeepMentionedAgentsPinned(true);
  audienceStore.promotePersistentAgentAudienceIfUnchanged({
    expectedRevision: audienceStore.getPersistentAgentAudienceRevision(scope),
    pubkeys: [agentA],
    scope,
  });
  audienceStore.promotePersistentAgentAudienceIfUnchanged({
    expectedRevision: audienceStore.getPersistentAgentAudienceRevision(scope),
    pubkeys: [agentB],
    source: "manual",
    scope,
  });

  preference.setKeepMentionedAgentsPinned(false);

  assert.deepEqual(
    audienceStore.getPersistentAgentAudienceSnapshot().audiences[scope],
    [agentB],
  );
});
