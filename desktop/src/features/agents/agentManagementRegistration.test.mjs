import assert from "node:assert/strict";
import test from "node:test";

import { registerAndRouteAgentManagementRequest } from "./agentManagementRegistration.ts";

const request = {
  type: "agent_management_request",
  action: "create",
  requestId: "70fc630c-a0ca-42e8-9392-326c5d4ef9c1",
  intent: "apply",
  request: {
    channelId: "7c07e659-3610-42f4-9a5e-1e9973c09da9",
    displayName: "Mintproof",
    systemPrompt: "Prove delegated creation.",
  },
};

test("authorized apply crosses the real register-to-publish seam without a draft result", async () => {
  const calls = [];
  const route = await registerAndRouteAgentManagementRequest(
    "11".repeat(32),
    request,
    {
      register: async (input) => {
        calls.push(["register", input]);
        return { disposition: "apply" };
      },
      publishApplying: async (input) => calls.push(["applying", input]),
      publishDraft: async (input) => calls.push(["draft", input]),
    },
  );

  assert.deepEqual(route, { kind: "delegated_apply" });
  assert.deepEqual(
    calls.map(([name]) => name),
    ["register"],
  );
  assert.equal(calls[0][1].capability, "agent_creation");
  assert.equal(calls[0][1].intent, "apply");
});

test("duplicate applying publishes a nonterminal management result", async () => {
  const calls = [];
  const route = await registerAndRouteAgentManagementRequest(
    "11".repeat(32),
    request,
    {
      register: async () => ({ disposition: "duplicate_applying" }),
      publishApplying: async (input) => calls.push(input),
      publishDraft: async () => assert.fail("draft result must not publish"),
    },
  );

  assert.deepEqual(route, { kind: "duplicate_applying" });
  assert.deepEqual(calls, [
    {
      requestId: request.requestId,
      sourcePubkey: "11".repeat(32),
    },
  ]);
});

test("revoked apply publishes queued-for-review only for the draft disposition", async () => {
  const calls = [];
  const route = await registerAndRouteAgentManagementRequest(
    "11".repeat(32),
    request,
    {
      register: async () => ({ disposition: "draft" }),
      publishApplying: async () =>
        assert.fail("applying result must not publish"),
      publishDraft: async (input) => calls.push(input),
    },
  );

  assert.deepEqual(route, { kind: "draft" });
  assert.equal(calls.length, 1);
});

test("env-only update registers against the narrow env_vars capability", async () => {
  const calls = [];
  const envUpdate = {
    ...request,
    action: "update",
    request: {
      channelId: request.request.channelId,
      agentName: "Goose colleague",
      envVars: { OPENAI_HOST: "http://lane" },
    },
  };
  const route = await registerAndRouteAgentManagementRequest(
    "11".repeat(32),
    envUpdate,
    {
      register: async (input) => {
        calls.push(input);
        return { disposition: "apply" };
      },
      publishApplying: async () => {},
      publishDraft: async () => {},
    },
  );
  assert.deepEqual(route, { kind: "delegated_apply" });
  assert.equal(calls[0].capability, "env_vars");
});
