import assert from "node:assert/strict";
import test from "node:test";

import { getProviderApiKeyFieldState } from "./providerApiKeyFieldState.ts";

test("providerApiKeyFieldState_missingAnthropicKey_isTopLevelRequired", () => {
  const state = getProviderApiKeyFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: {},
    envVars: {},
    globalEnvVars: {},
    provider: "anthropic",
    requiredEnvKeys: ["ANTHROPIC_API_KEY"],
  });

  assert.equal(state.secretEnvVar, "ANTHROPIC_API_KEY");
  assert.equal(state.isRequired, true);
  assert.equal(state.isInherited, false);
  assert.deepEqual(state.advancedRequiredEnvKeys, []);
});

test("providerApiKeyFieldState_globalCredential_isInheritedWithoutDuplicateAdvancedRow", () => {
  const state = getProviderApiKeyFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: {},
    envVars: {},
    globalEnvVars: { ANTHROPIC_API_KEY: "sk-global" },
    provider: "anthropic",
    requiredEnvKeys: [],
  });

  assert.equal(state.isRequired, false);
  assert.equal(state.isInherited, true);
  assert.equal(state.inheritedLabel, "Inherited from global config");
  assert.deepEqual(state.advancedRequiredEnvKeys, []);
});

test("providerApiKeyFieldState_personaCredential_isInheritedForInstanceTransition", () => {
  const state = getProviderApiKeyFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: { ANTHROPIC_API_KEY: "persona-secret" },
    envVars: {},
    globalEnvVars: {},
    personaSatisfied: true,
    provider: "anthropic",
    requiredEnvKeys: ["ANTHROPIC_API_KEY"],
  });

  assert.equal(state.isRequired, false);
  assert.equal(state.isInherited, true);
  assert.equal(state.inheritedLabel, "Inherited from agent profile");
});

test("providerApiKeyFieldState_explicitLocalEmptyShadowsGlobalAndBuild", () => {
  const state = getProviderApiKeyFieldState({
    bakedEnvKeys: ["ANTHROPIC_API_KEY"],
    effectiveEnvVars: { ANTHROPIC_API_KEY: "" },
    envVars: { ANTHROPIC_API_KEY: "" },
    globalEnvVars: { ANTHROPIC_API_KEY: "sk-global" },
    provider: "anthropic",
    requiredEnvKeys: ["ANTHROPIC_API_KEY"],
  });

  assert.equal(state.isRequired, true);
  assert.equal(state.isInherited, false);
  assert.equal(state.inheritedLabel, "");
});

test("providerApiKeyFieldState_openaiUsesCompatCredentialKey", () => {
  const state = getProviderApiKeyFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: {},
    envVars: {},
    globalEnvVars: {},
    provider: "openai",
    requiredEnvKeys: ["OPENAI_COMPAT_API_KEY"],
  });

  assert.equal(state.secretEnvVar, "OPENAI_COMPAT_API_KEY");
  assert.equal(state.isRequired, true);
});

test("providerApiKeyFieldState_explicitLocalEmptyShadowsRuntimeConfig", () => {
  const state = getProviderApiKeyFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: { ANTHROPIC_API_KEY: "" },
    envVars: { ANTHROPIC_API_KEY: "" },
    fileSatisfiedEnvKeys: ["ANTHROPIC_API_KEY"],
    globalEnvVars: {},
    provider: "anthropic",
    requiredEnvKeys: ["ANTHROPIC_API_KEY"],
  });

  assert.equal(state.isRequired, true);
  assert.equal(state.isInherited, false);
});

test("saved connection suppresses hosted secret controls without changing credentials", () => {
  const env = {
    BUZZ_MODEL_CONNECTION_ID: "saved",
    ANTHROPIC_API_KEY: "inactive",
  };
  const result = getProviderApiKeyFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: env,
    envVars: env,
    globalEnvVars: {},
    provider: "anthropic",
    requiredEnvKeys: ["ANTHROPIC_API_KEY", "UNRELATED_REQUIREMENT"],
  });
  assert.equal(result.secretEnvVar, null);
  assert.equal(result.isRequired, false);
  assert.deepEqual(result.advancedRequiredEnvKeys, ["UNRELATED_REQUIREMENT"]);
  assert.equal(env.ANTHROPIC_API_KEY, "inactive");
});

test("inherited source suppresses hosted credentials until explicitly cleared", () => {
  const common = {
    bakedEnvKeys: [],
    envVars: {},
    globalEnvVars: { BUZZ_MODEL_CONNECTION_ID: "inherited" },
    provider: "anthropic",
    requiredEnvKeys: ["ANTHROPIC_API_KEY"],
  };
  assert.equal(
    getProviderApiKeyFieldState({ ...common, effectiveEnvVars: {} })
      .secretEnvVar,
    null,
  );
  assert.equal(
    getProviderApiKeyFieldState({
      ...common,
      effectiveEnvVars: { BUZZ_MODEL_CONNECTION_ID: "" },
    }).secretEnvVar,
    "ANTHROPIC_API_KEY",
  );
});
