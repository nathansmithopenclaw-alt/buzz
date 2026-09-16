import assert from "node:assert/strict";
import test from "node:test";

import {
  NECTAR_AVATAR_PATH,
  NECTAR_DESCRIPTION,
  NECTAR_DISPLAY_NAME,
  NECTAR_PERSONA_ID,
  NECTAR_SYSTEM_PROMPT,
  ensureNectarReporter,
  findNectarAgent,
  findNectarPersona,
  isNectarPersonaId,
  nectarCreateInput,
  readNectarAvatarDataUrl,
} from "./nectarPersona.ts";

test("Nectar is a short bee-vein name like Honey, Fizz, and Pollen", () => {
  assert.equal(NECTAR_DISPLAY_NAME, "Nectar");
  assert.equal(NECTAR_DISPLAY_NAME.length <= 8, true);
  assert.equal(NECTAR_AVATAR_PATH, "/onboarding/starter-team/nectar.png");
  assert.match(NECTAR_DESCRIPTION, /digest/i);
});

test("Nectar's prompt assigns the Summary job and matches the starter-team voice", () => {
  assert.match(NECTAR_SYSTEM_PROMPT, /^You are Nectar/);
  assert.match(NECTAR_SYSTEM_PROMPT, /Summary/);
  assert.match(NECTAR_SYSTEM_PROMPT, /watermark/);
  assert.match(NECTAR_SYSTEM_PROMPT, /markdown/);
  assert.match(NECTAR_SYSTEM_PROMPT, /JSON/);
  assert.match(NECTAR_SYSTEM_PROMPT, /🐝/);
  assert.match(NECTAR_SYSTEM_PROMPT, /Do not invent work/);
});

test("isNectarPersonaId matches the stable starter id, not a display name", () => {
  assert.equal(isNectarPersonaId(NECTAR_PERSONA_ID), true);
  assert.equal(isNectarPersonaId("builtin:fizz"), false);
  assert.equal(isNectarPersonaId("Nectar"), false);
  assert.equal(isNectarPersonaId(null), false);
});

test("findNectarAgent and findNectarPersona match personaId, not display name", () => {
  assert.equal(
    findNectarAgent([
      { name: "Nectar", pubkey: "impostor", personaId: "other" },
      { name: "Digest", pubkey: "n", personaId: NECTAR_PERSONA_ID },
    ])?.pubkey,
    "n",
  );
  assert.equal(
    findNectarPersona([
      { id: "honey", displayName: "Nectar" },
      { id: NECTAR_PERSONA_ID, displayName: "Digest" },
    ])?.id,
    NECTAR_PERSONA_ID,
  );
  assert.equal(
    findNectarAgent([{ name: "Nectar", pubkey: "f", personaId: "fizz" }]),
    undefined,
  );
});

test("nectarCreateInput supplies identity and leaves harness/model to the user", () => {
  const input = nectarCreateInput({
    avatarUrl: "data:image/png;base64,abc",
    runtime: "claude",
    model: "opus",
    provider: "anthropic",
  });
  assert.equal(input.personaId, NECTAR_PERSONA_ID);
  assert.equal(input.displayName, "Nectar");
  assert.equal(input.systemPrompt, NECTAR_SYSTEM_PROMPT);
  assert.equal(input.runtime, "claude");
  assert.equal(input.model, "opus");
  assert.equal(input.provider, "anthropic");
  assert.deepEqual(input.namePool, ["Nectar"]);
});

test("ensureNectarReporter reuses an existing Nectar agent", async () => {
  let created = 0;
  const pubkey = await ensureNectarReporter({
    agents: [{ name: "Nectar", pubkey: "abc", personaId: NECTAR_PERSONA_ID }],
    personas: [],
    runtime: availableRuntime(),
    avatarDataUrl: "data:image/png;base64,abc",
    createPersona: async () => {
      created += 1;
      throw new Error("should not create");
    },
    createManagedAgent: async () => {
      created += 1;
      throw new Error("should not create");
    },
  });
  assert.equal(pubkey, "abc");
  assert.equal(created, 0);
});

test("ensureNectarReporter creates the persona then deploys it", async () => {
  const calls = [];
  const pubkey = await ensureNectarReporter({
    agents: [{ name: "Fizz", pubkey: "fizz" }],
    personas: [],
    runtime: availableRuntime(),
    model: "sonnet",
    provider: "anthropic",
    avatarDataUrl: "data:image/png;base64,abc",
    createPersona: async (input) => {
      calls.push(["persona", input.runtime, input.model]);
      return personaRecord({
        id: input.personaId ?? "p1",
        runtime: input.runtime ?? null,
      });
    },
    createManagedAgent: async (input) => {
      calls.push(["agent", input.personaId, input.name]);
      return { agent: { pubkey: "nectar-pk" } };
    },
    toInstanceInput: async (persona) => ({
      name: persona.displayName,
      personaId: persona.id,
    }),
  });
  assert.equal(pubkey, "nectar-pk");
  assert.deepEqual(calls, [
    ["persona", "claude", "sonnet"],
    ["agent", "nectar", "Nectar"],
  ]);
});

test("ensureNectarReporter deploys an existing Nectar persona", async () => {
  let createdPersona = false;
  const pubkey = await ensureNectarReporter({
    agents: [],
    personas: [personaRecord({ id: NECTAR_PERSONA_ID, runtime: "goose" })],
    runtime: availableRuntime(),
    avatarDataUrl: "data:image/png;base64,abc",
    createPersona: async () => {
      createdPersona = true;
      throw new Error("should not create persona");
    },
    updatePersona: async (input) => personaRecord({ id: input.id }),
    createManagedAgent: async () => ({ agent: { pubkey: "deployed" } }),
    toInstanceInput: async (persona) => ({
      name: persona.displayName,
      personaId: persona.id,
    }),
  });
  assert.equal(createdPersona, false);
  assert.equal(pubkey, "deployed");
});

test("readNectarAvatarDataUrl encodes the bundled PNG", async () => {
  const bytes = new Uint8Array([1, 2, 3, 4]);
  const dataUrl = await readNectarAvatarDataUrl(async (url) => {
    assert.equal(url, NECTAR_AVATAR_PATH);
    return new Response(bytes, {
      status: 200,
      headers: { "Content-Type": "image/png" },
    });
  });
  assert.equal(dataUrl.startsWith("data:image/png;base64,"), true);
});

function availableRuntime() {
  return {
    id: "claude",
    label: "Claude Code",
    avatarUrl: "",
    availability: "available",
    command: "claude",
    binaryPath: "/usr/bin/claude",
    defaultArgs: [],
    mcpCommand: null,
    modelEnvVar: null,
    providerEnvVar: null,
    thinkingEnvVar: null,
    effortModelQualifier: false,
    maxTokensEnvVar: null,
    contextLimitEnvVar: null,
    maxRoundsEnvVar: null,
    installHint: "",
    installInstructionsUrl: "",
    canAutoInstall: false,
    requiresExternalCli: false,
    underlyingCliPath: null,
    nodeRequired: false,
    authStatus: "authenticated",
    loginHint: null,
    source: "builtin",
  };
}

function personaRecord(overrides) {
  return {
    id: "p",
    displayName: "Nectar",
    avatarUrl: "data:image/png;base64,abc",
    systemPrompt: NECTAR_SYSTEM_PROMPT,
    runtime: "claude",
    model: null,
    provider: null,
    namePool: ["Nectar"],
    isBuiltIn: false,
    isActive: true,
    shared: false,
    catalogSource: null,
    envVars: {},
    respondTo: null,
    respondToAllowlist: [],
    parallelism: null,
    createdAt: "",
    updatedAt: "",
    ...overrides,
  };
}

test("Nectar creation persists the selected model and tuning on the instance", async () => {
  let saved;
  const envVars = {
    BUZZ_ACP_FAST_MODE: "true",
    CLAUDE_CODE_EFFORT_LEVEL: "high",
  };
  await ensureNectarReporter({
    agents: [],
    personas: [],
    runtime: availableRuntime(),
    model: "opus",
    envVars,
    avatarDataUrl: "data:image/png;base64,abc",
    createPersona: async (input) =>
      personaRecord({ ...input, id: NECTAR_PERSONA_ID }),
    createManagedAgent: async (input) => {
      saved = input;
      return { agent: { pubkey: "reporter" } };
    },
    toInstanceInput: async () => ({ name: "Nectar", envVars: {} }),
  });
  assert.equal(saved.model, "opus");
  assert.deepEqual(saved.envVars, envVars);
});
