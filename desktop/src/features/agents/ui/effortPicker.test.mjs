import assert from "node:assert/strict";
import test from "node:test";

import {
  EFFORT_DEFAULT_DROPDOWN_VALUE,
  effortPickerState,
  discoveredEffortCatalog,
  effortDiscoveryState,
  unrenderedEffortExplanation,
  validateSavedEffort,
  effortSelectionToPersistedValue,
} from "./effortPicker.ts";
import { deriveAgentConfigFieldModel } from "../lib/agentConfigCore.ts";

const localBackend = { type: "local" };
const providerBackend = { type: "provider", id: "openai", config: {} };
const options = [
  { value: "low", displayName: "Low" },
  { value: "high", displayName: "High" },
];

test("effort picker renders for a local backend with a discovered configId", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortConfigId: "thought_level",
    effortOptions: options,
    currentEffort: null,
  });
  assert.equal(state.visible, true);
});

test("effort picker is hidden for a provider backend even when a configId exists", () => {
  const state = effortPickerState({
    backend: providerBackend,
    effortConfigId: "thought_level",
    effortOptions: options,
    currentEffort: "high",
  });
  assert.equal(state.visible, false);
});

test("effort picker is hidden for a local backend without a discovered configId", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortConfigId: undefined,
    effortOptions: undefined,
    currentEffort: null,
  });
  assert.equal(state.visible, false);
});

test("options lead with the adapter-default sentinel then adapter values", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortConfigId: "thought_level",
    effortOptions: options,
    currentEffort: null,
  });
  assert.deepEqual(state.options, [
    { label: "Inherit defaults", value: EFFORT_DEFAULT_DROPDOWN_VALUE },
    { label: "Low", value: "low" },
    { label: "High", value: "high" },
  ]);
});

test("option label falls back to the raw value when displayName is absent", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortConfigId: "thought_level",
    effortOptions: [{ value: "medium" }],
    currentEffort: null,
  });
  assert.deepEqual(state.options[1], { label: "medium", value: "medium" });
});

test("current effort preselects the matching option", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortConfigId: "thought_level",
    effortOptions: options,
    currentEffort: "high",
  });
  assert.equal(state.selectValue, "high");
});

test("an unavailable saved effort stays visible without becoming the default", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortConfigId: "thought_level",
    effortOptions: options,
    currentEffort: "extreme",
  });
  assert.equal(state.selectValue, "extreme");
  assert.deepEqual(state.options.at(-1), {
    label: "extreme (unavailable)",
    value: "extreme",
    disabled: true,
  });
});

test("a null current effort selects the adapter-default sentinel", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortConfigId: "thought_level",
    effortOptions: options,
    currentEffort: null,
  });
  assert.equal(state.selectValue, EFFORT_DEFAULT_DROPDOWN_VALUE);
});

test("the sentinel selection persists as null (clear to adapter default)", () => {
  assert.equal(
    effortSelectionToPersistedValue(EFFORT_DEFAULT_DROPDOWN_VALUE),
    null,
  );
});

test("a concrete selection persists as its explicit effort level", () => {
  assert.equal(effortSelectionToPersistedValue("high"), "high");
});

test("a model change rejects an unsupported effort without silently clearing it", () => {
  assert.match(
    validateSavedEffort("ultra", true, [{ id: "low" }]),
    /not supported/,
  );
  assert.equal(validateSavedEffort("low", true, [{ id: "low" }]), null);
  assert.equal(validateSavedEffort(null, true, [{ id: "low" }]), null);
});
test("unrelated edits and failed discovery preserve the saved effort", () => {
  assert.equal(validateSavedEffort("ultra", false, []), null);
  assert.equal(validateSavedEffort("ultra", true, null), null);
});

test("discovery with no effort control is an unknown catalog, not an empty one", () => {
  // A runtime that advertises no thought_level returns effortConfigId null and
  // an empty option list. Reading that as "zero valid levels" rejected every
  // level the picker offered from the session config surface and wedged Save.
  assert.equal(discoveredEffortCatalog(null, []), null);
  assert.equal(discoveredEffortCatalog(undefined, []), null);
  assert.equal(
    validateSavedEffort("high", true, discoveredEffortCatalog(null, [])),
    null,
  );
  // A real discovered catalog still validates.
  assert.deepEqual(discoveredEffortCatalog("depth", [{ id: "low" }]), [
    { id: "low" },
  ]);
  assert.match(
    validateSavedEffort(
      "high",
      true,
      discoveredEffortCatalog("depth", [{ id: "low" }]),
    ),
    /not supported/,
  );
});

function unrenderedFieldModel(runtimeMetadata) {
  return deriveAgentConfigFieldModel({
    config: { env_vars: {}, model: "", provider: "" },
    runtime: {
      id: runtimeMetadata.id,
      providerEnvVar: null,
      modelEnvVar: null,
      thinkingEnvVar: null,
      maxTokensEnvVar: null,
      contextLimitEnvVar: null,
      maxRoundsEnvVar: null,
      ...runtimeMetadata,
    },
    scope: "definition",
  });
}

test("effort discovery state separates pending, failed, absent, and offered", () => {
  const cases = [
    [{ pending: true, discovered: false, effortOptions: null }, "pending"],
    // A stale response must not read as settled while a new probe runs.
    [{ pending: true, discovered: true, effortOptions: [{}] }, "pending"],
    [{ pending: false, discovered: false, effortOptions: null }, "unavailable"],
    [{ pending: false, discovered: true, effortOptions: null }, "none"],
    [{ pending: false, discovered: true, effortOptions: [] }, "none"],
    [{ pending: false, discovered: true, effortOptions: [{}] }, "available"],
  ];
  for (const [input, expected] of cases) {
    assert.equal(effortDiscoveryState(input), expected, JSON.stringify(input));
  }
});

test("a deferred Claude effort control names its state and never claims the harness lacks effort", () => {
  // Claude effort is a deferred harness-native descriptor, not an omission.
  const claude = unrenderedFieldModel({ id: "claude" });
  const local = [
    ["pending", /Checking which thinking levels/],
    ["unavailable", /couldn’t load thinking levels/],
    ["none", /doesn’t offer thinking levels/],
    ["available", /set on each deployed agent/],
  ];
  for (const runLocation of [null, "local"]) {
    for (const [state, expected] of local) {
      const copy = unrenderedEffortExplanation(claude, state, runLocation);
      assert.match(copy, expected, `${state} ${runLocation}`);
      assert.doesNotMatch(copy, /does not advertise/, state);
    }
  }
  // Remote effort is deployment policy: no local instruction, in any state.
  for (const [state] of local) {
    const copy = unrenderedEffortExplanation(claude, state, "remote");
    assert.match(copy, /runs on a server/, state);
    assert.doesNotMatch(copy, /choose Edit/, state);
  }
});

test("omitted effort keeps its named-reason copy", () => {
  assert.match(
    unrenderedEffortExplanation(
      unrenderedFieldModel({ id: "codex", effortModelQualifier: false }),
      "unavailable",
      null,
    ),
    /model_reasoning_effort/,
  );
  // An unsupported harness stays unsupported even when a probe reported levels.
  assert.match(
    unrenderedEffortExplanation(
      unrenderedFieldModel({ id: "amp" }),
      "available",
      "local",
    ),
    /does not advertise or expose a reasoning-effort setting/,
  );
});
