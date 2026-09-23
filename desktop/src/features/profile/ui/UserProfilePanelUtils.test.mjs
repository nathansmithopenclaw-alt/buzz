import assert from "node:assert/strict";
import test from "node:test";

import {
  linkedAgentUpdateAfterDefinitionSave,
  resolvePanelProfile,
  parseProfilePanelTab,
  parseProfilePanelView,
  personaManagedAgentUpdate,
  profilePanelTabFromSearch,
  profilePanelTargetKey,
  profilePanelViewFromSearch,
} from "./UserProfilePanelUtils.ts";

function agent(overrides = {}) {
  return {
    pubkey: "deadbeef".repeat(8),
    name: "Fizz",
    personaId: "persona-1",
    relayUrl: "ws://localhost:3000",
    acpCommand: "buzz-acp",
    agentCommand: "goose",
    agentArgs: [],
    mcpCommand: "",
    turnTimeoutSeconds: 320,
    idleTimeoutSeconds: null,
    maxTurnDurationSeconds: null,
    parallelism: 1,
    systemPrompt: "Old prompt",
    avatarUrl: "app-avatar://old",
    model: "old-model",
    envVars: { OLD_KEY: "1" },
    status: "stopped",
    pid: null,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    lastStartedAt: null,
    lastStoppedAt: null,
    lastExitCode: null,
    lastError: null,
    logPath: null,
    startOnAppLaunch: true,
    backend: { type: "local" },
    backendAgentId: null,
    respondTo: "owner-only",
    respondToAllowlist: [],
    ...overrides,
  };
}

function persona(overrides = {}) {
  return {
    id: "persona-1",
    displayName: "Fizz Prime",
    avatarUrl: null,
    systemPrompt: "New prompt",
    runtime: "goose",
    model: "new-model",
    provider: null,
    namePool: [],
    isBuiltIn: false,
    isActive: true,
    respondTo: "owner-only",
    respondToAllowlist: [],
    envVars: { NEW_KEY: "2" },
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function runtime(overrides = {}) {
  return {
    id: "claude",
    label: "Claude Code",
    avatarUrl: "app-avatar://claude",
    availability: "available",
    command: "claude",
    binaryPath: "/usr/local/bin/claude",
    defaultArgs: ["mcp", "serve"],
    mcpCommand: "claude-mcp",
    installHint: "",
    installInstructionsUrl: "",
    canAutoInstall: false,
    underlyingCliPath: null,
    ...overrides,
  };
}

test("personaManagedAgentUpdate syncs edited persona identity to linked agent", () => {
  assert.deepEqual(personaManagedAgentUpdate(agent(), persona()), {
    pubkey: "deadbeef".repeat(8),
    name: "Fizz Prime",
    systemPrompt: "New prompt",
    model: "new-model",
    envVars: { NEW_KEY: "2" },
  });
});

test("personaManagedAgentUpdate syncs definition access to the linked agent", () => {
  assert.deepEqual(
    personaManagedAgentUpdate(
      agent({ respondTo: "anyone" }),
      persona({ respondTo: "owner-only" }),
    ),
    {
      pubkey: "deadbeef".repeat(8),
      name: "Fizz Prime",
      systemPrompt: "New prompt",
      model: "new-model",
      envVars: { NEW_KEY: "2" },
      respondTo: "owner-only",
    },
  );

  assert.deepEqual(
    personaManagedAgentUpdate(
      agent({ respondTo: "anyone" }),
      persona({
        respondTo: "allowlist",
        respondToAllowlist: ["a".repeat(64)],
      }),
    ),
    {
      pubkey: "deadbeef".repeat(8),
      name: "Fizz Prime",
      systemPrompt: "New prompt",
      model: "new-model",
      envVars: { NEW_KEY: "2" },
      respondTo: "allowlist",
      respondToAllowlist: ["a".repeat(64)],
    },
  );
});

test("personaManagedAgentUpdate skips unrelated or unchanged agents", () => {
  assert.equal(
    personaManagedAgentUpdate(agent({ personaId: "persona-2" }), persona()),
    null,
  );
  assert.equal(
    personaManagedAgentUpdate(
      agent({
        name: "Fizz Prime",
        avatarUrl: null,
        systemPrompt: "New prompt",
        model: "new-model",
        envVars: { NEW_KEY: "2" },
      }),
      persona(),
    ),
    null,
  );
});

test("personaManagedAgentUpdate maps changed persona runtime to linked agent commands", () => {
  assert.deepEqual(
    personaManagedAgentUpdate(agent(), persona({ runtime: "claude" }), {
      previousPersona: persona({ runtime: "goose" }),
      runtimes: [runtime()],
    }),
    {
      pubkey: "deadbeef".repeat(8),
      name: "Fizz Prime",
      systemPrompt: "New prompt",
      model: "new-model",
      envVars: { NEW_KEY: "2" },
      agentCommand: "claude",
      agentArgs: ["mcp", "serve"],
      mcpCommand: "claude-mcp",
    },
  );
});

test("personaManagedAgentUpdate leaves runtime fields alone when runtime is unchanged", () => {
  assert.equal(
    personaManagedAgentUpdate(
      agent({
        name: "Fizz Prime",
        avatarUrl: null,
        systemPrompt: "New prompt",
        model: "new-model",
        envVars: { NEW_KEY: "2" },
        agentArgs: ["custom"],
      }),
      persona({ runtime: "goose" }),
      {
        previousPersona: persona({ runtime: "goose" }),
        runtimes: [runtime({ id: "goose", command: "goose" })],
      },
    ),
    null,
  );
});

test("parseProfilePanelView accepts all profile panel subviews", () => {
  for (const view of [
    "summary",
    "activity",
    "info",
    "configuration",
    "diagnostics",
    "memories",
    "channels",
    "logs",
  ]) {
    assert.equal(parseProfilePanelView(view), view);
  }
});

test("parseProfilePanelView maps legacy agent config subviews to configuration", () => {
  for (const view of ["model", "settings"]) {
    assert.equal(parseProfilePanelView(view), "configuration");
  }
});

test("profilePanelViewFromSearch falls back to summary for invalid values", () => {
  assert.equal(parseProfilePanelView("missing"), null);
  assert.equal(profilePanelViewFromSearch("missing"), "summary");
  assert.equal(profilePanelViewFromSearch(null), "summary");
});

test("parseProfilePanelTab accepts profile summary tabs", () => {
  for (const tab of ["info", "runtime", "channels", "memories"]) {
    assert.equal(parseProfilePanelTab(tab), tab);
  }
});

test("profilePanelTabFromSearch falls back to info for invalid values", () => {
  assert.equal(parseProfilePanelTab("missing"), null);
  assert.equal(profilePanelTabFromSearch("missing"), "info");
  assert.equal(profilePanelTabFromSearch(null), "info");
});

test("profile target identity stays stable while a requested pubkey is canonicalized", () => {
  const historicalPubkey = "a".repeat(64);
  assert.equal(
    profilePanelTargetKey(historicalPubkey, undefined),
    historicalPubkey,
  );
  assert.equal(
    profilePanelTargetKey(historicalPubkey, "resolved-persona"),
    historicalPubkey,
  );
  assert.equal(
    profilePanelTargetKey(undefined, "requested-persona"),
    "persona:requested-persona",
  );
});

test("managed profile uses the exact instance selection and preserves removal", () => {
  for (const avatarUrl of ["https://example.com/selected.png", ""]) {
    const result = resolvePanelProfile({
      managedAgent: agent({ avatarUrl }),
      persona: persona({ avatarUrl: "https://example.com/definition.png" }),
      profile: {
        pubkey: agent().pubkey,
        avatarUrl: "https://example.com/stale.png",
        displayName: "Relay",
      },
    });
    assert.equal(result.avatarUrl, avatarUrl || null);
    assert.equal(result.displayName, "Relay");
  }
});

// The agent is owner-only while its definition's mint-time default is
// "anyone". Every other synced field matches.
function narrowerThanDefinition() {
  const shared = {
    systemPrompt: "Same prompt",
    model: "same-model",
    envVars: {},
  };
  return {
    agent: agent({ ...shared, respondTo: "owner-only" }),
    savedPersona: persona({
      ...shared,
      displayName: "Fizz",
      respondTo: "anyone",
    }),
  };
}

test("linkedAgentUpdateAfterDefinitionSave never copies an untouched definition access over the agent", () => {
  const { agent: linked, savedPersona } = narrowerThanDefinition();
  assert.equal(
    linkedAgentUpdateAfterDefinitionSave(linked, savedPersona, {
      accessSubmitted: false,
    }),
    null,
    "an unchanged Save writes nothing to the agent",
  );
  assert.deepEqual(
    linkedAgentUpdateAfterDefinitionSave(linked, savedPersona, {
      accessSubmitted: false,
      effortLevel: "high",
    }),
    { pubkey: linked.pubkey, effortLevel: "high" },
    "an effort-only Save sends only the effort",
  );
  assert.deepEqual(
    linkedAgentUpdateAfterDefinitionSave(
      linked,
      { ...savedPersona, model: "new-model" },
      { accessSubmitted: false },
    ),
    { pubkey: linked.pubkey, model: "new-model" },
    "other synced fields still reach the agent without its access",
  );
});

test("linkedAgentUpdateAfterDefinitionSave syncs access the dialog submitted", () => {
  const { agent: linked, savedPersona } = narrowerThanDefinition();
  assert.deepEqual(
    linkedAgentUpdateAfterDefinitionSave(linked, savedPersona, {
      accessSubmitted: true,
      effortLevel: null,
    }),
    { pubkey: linked.pubkey, respondTo: "anyone", effortLevel: null },
  );
});
