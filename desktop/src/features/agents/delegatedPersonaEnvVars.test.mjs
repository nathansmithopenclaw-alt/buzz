import assert from "node:assert/strict";
import test from "node:test";

import {
  applyDelegatedPersonaEnvVarsWith,
  delegatedEnvActivationCounts,
  delegatedEnvAuditTarget,
} from "./delegatedPersonaEnvVars.ts";

test("delegated env apply sends only request keys and the resolved persona id", async () => {
  const calls = [];
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

  const result = await applyDelegatedPersonaEnvVarsWith(
    request,
    "persona-sparkproof",
    {
      applyPatch: async (patch) => {
        calls.push(patch);
        return {
          persona: { id: patch.id },
          restarted_count: 1,
          started_count: 0,
          failed_start_count: 0,
        };
      },
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
