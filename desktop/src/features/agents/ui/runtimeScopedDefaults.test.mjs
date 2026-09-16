import assert from "node:assert/strict";
import { test } from "node:test";
import { getInheritedAgentDefaults } from "./bakedEnvHelpers.ts";
import { getDefaultLlmModelLabel } from "./agentConfigOptions.tsx";

const global = {
  preferred_runtime: "claude",
  model: "claude-fable-5[1m]",
  provider: "anthropic",
  env_vars: { BUZZ_AGENT_THINKING_EFFORT: "high" },
};

test("dialog defaults and label do not inherit selectors across runtimes", () => {
  for (const runtime of ["cursor", "codex", "prime", "custom-acp"]) {
    const defaults = getInheritedAgentDefaults(global, undefined, runtime);
    assert.equal(defaults.model.value, "", runtime);
    assert.equal(defaults.provider.value, "", runtime);
    assert.equal(
      getDefaultLlmModelLabel(defaults.model.value),
      "Default model",
    );
    assert.equal(defaults.effort.value, "high");
  }
  const defaults = getInheritedAgentDefaults(global, undefined, "claude");
  assert.equal(defaults.model.value, global.model);
  assert.equal(defaults.provider.value, global.provider);
  assert.equal(
    getDefaultLlmModelLabel(defaults.model.value),
    `Use agent defaults (${global.model})`,
  );
});

test("legacy globals without a preference apply to the bundled runtime only", () => {
  const legacy = { ...global, preferred_runtime: null };
  assert.equal(
    getInheritedAgentDefaults(legacy, undefined, "buzz-agent").model.value,
    global.model,
  );
  assert.equal(
    getInheritedAgentDefaults(legacy, undefined, "cursor").model.value,
    "",
  );
  assert.equal(
    getInheritedAgentDefaults(legacy, undefined).model.value,
    global.model,
  );
});
