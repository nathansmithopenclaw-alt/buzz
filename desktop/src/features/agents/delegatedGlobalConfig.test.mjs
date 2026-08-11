import assert from "node:assert/strict";
import test from "node:test";

import { applyDelegatedGlobalConfigWith } from "./delegatedGlobalConfig.ts";

const request = {
  type: "agent_management_request",
  action: "global_config",
  requestId: "request-global-config",
  intent: "apply",
  request: {
    channelId: "7c07e659-3610-42f4-9a5e-1e9973c09da9",
    preferredRuntime: "codex",
    effort: "high",
  },
};

test("global apply reloads durable config and overlays only requested fields", async () => {
  const grant = { granteePubkey: "11".repeat(32) };
  const saved = [];
  const result = await applyDelegatedGlobalConfigWith(request, {
    getConfig: async () => ({
      env_vars: { PRESERVED: "yes" },
      provider: "existing-provider",
      model: "existing-model",
      preferred_runtime: null,
      auto_approve_agent_drafts: false,
      agent_management_delegation: grant,
    }),
    discoverRuntimes: async () => [
      {
        id: "codex",
        availability: "available",
        thinkingEnvVar: "BUZZ_ACP_THINKING_EFFORT",
      },
    ],
    applyPatch: async (patch) => {
      saved.push(patch);
      const config = {
        env_vars: { PRESERVED: "yes", ...patch.envVars },
        provider: "existing-provider",
        model: "existing-model",
        preferred_runtime: patch.preferredRuntime,
        auto_approve_agent_drafts: false,
        agent_management_delegation: grant,
      };
      return {
        config,
        restarted_count: 0,
        failed_restart_count: 0,
      };
    },
  });

  assert.equal(saved.length, 1);
  assert.deepEqual(saved[0], {
    preferredRuntime: "codex",
    envVars: { BUZZ_ACP_THINKING_EFFORT: "high" },
  });
  assert.equal(result.config.env_vars.PRESERVED, "yes");
  assert.equal(result.config.env_vars.BUZZ_ACP_THINKING_EFFORT, "high");
  assert.equal(result.config.provider, "existing-provider");
  assert.equal(result.config.model, "existing-model");
  assert.equal(result.config.preferred_runtime, "codex");
  assert.equal(result.config.agent_management_delegation, grant);
});

test("global effort rejects an explicit runtime without declared effort support", async () => {
  let setCalled = false;
  await assert.rejects(
    applyDelegatedGlobalConfigWith(request, {
      getConfig: async () => ({
        env_vars: {},
        provider: null,
        model: null,
        preferred_runtime: null,
        auto_approve_agent_drafts: false,
        agent_management_delegation: null,
      }),
      discoverRuntimes: async () => [
        { id: "codex", availability: "available", thinkingEnvVar: null },
      ],
      applyPatch: async () => {
        setCalled = true;
        throw new Error("must not save");
      },
    }),
    /does not support effort configuration/,
  );
  assert.equal(setCalled, false);
});
