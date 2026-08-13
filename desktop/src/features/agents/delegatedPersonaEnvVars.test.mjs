import assert from "node:assert/strict";
import test from "node:test";

import {
  applyDelegatedPersonaEnvVarsWith,
  delegatedEnvActivationCounts,
  delegatedEnvAuditTarget,
  mergeResurrectionIntoActivation,
} from "./delegatedPersonaEnvVars.ts";

const request = {
  type: "agent_management_request",
  action: "update",
  requestId: "request-env-update",
  intent: "apply",
  request: {
    channelId: "7c07e659-3610-42f4-9a5e-1e9973c09da9",
    agentName: "Sparkproof",
    envVars: { OPENAI_COMPAT_API_KEY: "delegated-value" },
  },
};

function envApplyResult(overrides = {}) {
  return {
    persona: { id: "persona-sparkproof" },
    restarted_count: 0,
    started_count: 0,
    failed_start_count: 0,
    ...overrides,
  };
}

test("delegated env apply sends only request keys and the resolved persona id", async () => {
  const calls = [];
  const result = await applyDelegatedPersonaEnvVarsWith(
    request,
    "persona-sparkproof",
    {
      applyPatch: async (patch) => {
        calls.push(patch);
        return envApplyResult({
          persona: { id: patch.id },
          restarted_count: 1,
        });
      },
      resurrectStopped: async () => ({ started: [], failed: [] }),
    },
  );

  assert.deepEqual(calls, [
    {
      id: "persona-sparkproof",
      envVars: { OPENAI_COMPAT_API_KEY: "delegated-value" },
    },
  ]);
  assert.equal(result.restarted_count, 1);
  const counts = delegatedEnvActivationCounts(result);
  assert.deepEqual(counts, {
    restartedCount: 1,
    startedCount: 0,
    failedStartCount: 0,
  });
  assert.equal(
    delegatedEnvAuditTarget("Sparkproof", counts),
    "saved env for agent Sparkproof; applied now: started 0, restarted 1",
  );
  assert.equal(
    delegatedEnvAuditTarget("Sparkproof", {}),
    "saved env for agent Sparkproof; changes take effect on next spawn",
  );
  assert.equal(
    delegatedEnvAuditTarget("Sparkproof", { failedStartCount: 1 }),
    "saved env for agent Sparkproof; activation incomplete: started 0, restarted 0, failed 1",
  );
});

test("delegated env apply resurrects a stopped already-ready harness", async () => {
  const order = [];
  const result = await applyDelegatedPersonaEnvVarsWith(
    request,
    "persona-emberproof",
    {
      applyPatch: async (patch) => {
        order.push(["apply", patch.id]);
        return envApplyResult({ persona: { id: patch.id } });
      },
      resurrectStopped: async (input) => {
        order.push(["resurrect", input.personaId]);
        return { started: ["ee".repeat(32)], failed: [] };
      },
    },
  );

  assert.deepEqual(order, [
    ["apply", "persona-emberproof"],
    ["resurrect", "persona-emberproof"],
  ]);
  assert.deepEqual(delegatedEnvActivationCounts(result), {
    restartedCount: 0,
    startedCount: 1,
    failedStartCount: 0,
  });
  assert.equal(
    delegatedEnvAuditTarget("Emberproof", delegatedEnvActivationCounts(result)),
    "saved env for agent Emberproof; applied now: started 1, restarted 0",
  );
});

test("delegated env apply adds resurrection starts to readiness-transition counts", async () => {
  const result = await applyDelegatedPersonaEnvVarsWith(
    request,
    "persona-sparkproof",
    {
      applyPatch: async () =>
        envApplyResult({
          restarted_count: 1,
          started_count: 1,
          failed_start_count: 1,
        }),
      resurrectStopped: async () => ({
        started: ["aa".repeat(32)],
        failed: [`${"bb".repeat(32)}: spawn failed`],
      }),
    },
  );

  assert.deepEqual(delegatedEnvActivationCounts(result), {
    restartedCount: 1,
    startedCount: 2,
    failedStartCount: 2,
  });
});

test("delegated env apply logs a resurrection invoke failure and counts it", async () => {
  const errors = [];
  const originalError = console.error;
  console.error = (...args) => {
    errors.push(args);
  };
  try {
    const result = await applyDelegatedPersonaEnvVarsWith(
      request,
      "persona-emberproof",
      {
        applyPatch: async () => envApplyResult(),
        resurrectStopped: async () => {
          throw new Error("invoke failed: agent_is_running lock");
        },
      },
    );
    assert.deepEqual(delegatedEnvActivationCounts(result), {
      restartedCount: 0,
      startedCount: 0,
      failedStartCount: 1,
    });
    assert.match(
      String(errors[0]?.[0] ?? ""),
      /resurrect_stopped_active_instances failed after env apply/,
    );
  } finally {
    console.error = originalError;
  }
});

test("mergeResurrectionIntoActivation does not invent restarted counts", () => {
  assert.deepEqual(
    mergeResurrectionIntoActivation(envApplyResult({ restarted_count: 3 }), {
      started: ["aa".repeat(32), "cc".repeat(32)],
      failed: ["dd".repeat(32)],
    }),
    envApplyResult({
      restarted_count: 3,
      started_count: 2,
      failed_start_count: 1,
    }),
  );
});
