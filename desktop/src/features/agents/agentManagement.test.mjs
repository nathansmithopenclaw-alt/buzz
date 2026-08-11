import assert from "node:assert/strict";
import test from "node:test";

import {
  AGENT_MANAGEMENT_REQUEST,
  createInputFromRequest,
  globalConfigAuditTarget,
  globalConfigPatchFromRequest,
  globalConfigReviewRows,
  parseAgentManagementRequest,
  requestTargetsEditablePersona,
  updateInputFromRequest,
} from "./agentManagement.ts";

const CHANNEL_ID = "7c07e659-3610-42f4-9a5e-1e9973c09da9";

function createPayload(overrides = {}) {
  return {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "create",
    requestId: "request-1",
    intent: "draft",
    request: {
      channelId: CHANNEL_ID,
      displayName: "Research helper",
      systemPrompt: "Find reliable sources and summarize them.",
    },
    ...overrides,
  };
}

test("parses the narrow no-secret create request", () => {
  assert.deepEqual(
    parseAgentManagementRequest(createPayload()),
    createPayload(),
  );
});

test("defaults legacy requests to draft and preserves explicit apply intent", () => {
  const legacy = createPayload();
  delete legacy.intent;
  assert.equal(parseAgentManagementRequest(legacy)?.intent, "draft");
  assert.equal(
    parseAgentManagementRequest(createPayload({ intent: "apply" }))?.intent,
    "apply",
  );
});

test("rejects unknown management intent values", () => {
  assert.equal(
    parseAgentManagementRequest(createPayload({ intent: "owner-bypass" })),
    null,
  );
});

test("rejects an agent-management request with extra secret-shaped fields", () => {
  const payload = createPayload();
  payload.request.apiKey = "should-not-be-accepted";

  assert.equal(parseAgentManagementRequest(payload), null);
});

test("parses and maps full non-secret create configuration", () => {
  const payload = createPayload();
  Object.assign(payload.request, {
    runtime: "codex",
    provider: "openai",
    model: "gpt-5",
    respondTo: "anyone",
    parallelism: 4,
  });
  const parsed = parseAgentManagementRequest(payload);
  assert.ok(parsed && parsed.action === "create");

  assert.deepEqual(createInputFromRequest(parsed), {
    displayName: "Research helper",
    systemPrompt: "Find reliable sources and summarize them.",
    runtime: "codex",
    provider: "openai",
    model: "gpt-5",
    behavior: {
      respondTo: "anyone",
      parallelism: 4,
    },
  });
});

test("maps delegated env vars without losing values containing equals", () => {
  const payload = createPayload({ intent: "apply" });
  payload.request.envVars = {
    OPENAI_HOST: "http://100.80.202.75:4000",
    OPENAI_API_KEY: "dummy=with=equals",
  };
  const parsed = parseAgentManagementRequest(payload);
  assert.ok(parsed && parsed.action === "create");
  assert.deepEqual(
    createInputFromRequest(parsed).envVars,
    payload.request.envVars,
  );
});

test("rejects invalid create behavior values", () => {
  for (const [field, value] of [
    ["respondTo", "allowlist"],
    ["parallelism", 0],
    ["parallelism", 33],
    ["parallelism", 1.5],
  ]) {
    const payload = createPayload();
    payload.request[field] = value;
    assert.equal(parseAgentManagementRequest(payload), null);
  }
});

test("rejects invalid optional create text instead of silently dropping it", () => {
  for (const [field, value] of [
    ["runtime", 42],
    ["provider", null],
    ["model", {}],
    ["model", "   "],
  ]) {
    const payload = createPayload();
    payload.request[field] = value;
    assert.equal(parseAgentManagementRequest(payload), null);
  }
});

test("requires the originating channel for profile updates", () => {
  const payload = {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "update",
    requestId: "request-2",
    request: {
      agentName: "Review helper",
      systemPrompt: "Review changes concisely.",
    },
  };

  assert.equal(parseAgentManagementRequest(payload), null);
});

test("uses an agent's current name, never an internal profile ID", () => {
  const payload = {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "update",
    requestId: "request-3",
    intent: "draft",
    request: {
      channelId: CHANNEL_ID,
      agentName: "Review helper",
      systemPrompt: "Review changes concisely.",
    },
  };

  assert.deepEqual(parseAgentManagementRequest(payload), payload);
});

test("parses and maps parallelism plus instance name pool updates", () => {
  const payload = {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "update",
    requestId: "request-update-behavior",
    request: {
      channelId: CHANNEL_ID,
      agentName: "Review helper",
      parallelism: 6,
      namePool: ["Alpha", "Beta"],
    },
  };
  const parsed = parseAgentManagementRequest(payload);
  assert.ok(parsed && parsed.action === "update");

  assert.deepEqual(
    updateInputFromRequest(parsed, {
      id: "persona-1",
      displayName: "Review helper",
      systemPrompt: "Review changes.",
      namePool: ["Old"],
      behavior: {
        respondTo: "owner-only",
        respondToAllowlist: ["owner"],
        parallelism: 2,
      },
    }),
    {
      id: "persona-1",
      displayName: "Review helper",
      systemPrompt: "Review changes.",
      runtime: undefined,
      provider: undefined,
      model: undefined,
      namePool: ["Alpha", "Beta"],
      behavior: {
        respondTo: "owner-only",
        respondToAllowlist: ["owner"],
        parallelism: 6,
      },
    },
  );
});

test("accepts an empty instance name pool to clear it", () => {
  const payload = {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "update",
    requestId: "request-clear-name-pool",
    intent: "draft",
    request: {
      channelId: CHANNEL_ID,
      agentName: "Review helper",
      namePool: [],
    },
  };

  assert.deepEqual(parseAgentManagementRequest(payload), payload);
});

test("env-only update merges requested keys into the existing definition", () => {
  const payload = {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "update",
    requestId: "request-env-update",
    intent: "apply",
    request: {
      channelId: CHANNEL_ID,
      agentName: "Review helper",
      envVars: { OPENAI_HOST: "http://lane" },
    },
  };
  const parsed = parseAgentManagementRequest(payload);
  assert.ok(parsed && parsed.action === "update");
  const updated = updateInputFromRequest(parsed, {
    id: "persona-1",
    displayName: "Review helper",
    systemPrompt: "Review changes.",
    envVars: { PRESERVED: "yes" },
  });
  assert.deepEqual(updated.envVars, {
    PRESERVED: "yes",
    OPENAI_HOST: "http://lane",
  });
});

test("agent config update maps effort and a respond-to allowlist", () => {
  const payload = {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "update",
    requestId: "request-agent-config",
    intent: "apply",
    request: {
      channelId: CHANNEL_ID,
      agentName: "Review helper",
      systemPrompt: "Apply the approval clause.",
      effort: "high",
      respondTo: "allowlist",
      respondToAllowlist: ["11".repeat(32)],
    },
  };
  const parsed = parseAgentManagementRequest(payload);
  assert.ok(parsed && parsed.action === "update");
  const updated = updateInputFromRequest(
    parsed,
    {
      id: "persona-1",
      displayName: "Review helper",
      systemPrompt: "Old instructions.",
      envVars: { PRESERVED: "yes" },
      behavior: { respondTo: "owner-only", respondToAllowlist: [] },
    },
    "BUZZ_ACP_THINKING_EFFORT",
  );
  assert.equal(updated.systemPrompt, "Apply the approval clause.");
  assert.equal(updated.envVars.BUZZ_ACP_THINKING_EFFORT, "high");
  assert.deepEqual(updated.behavior, {
    respondTo: "allowlist",
    respondToAllowlist: ["11".repeat(32)],
    parallelism: undefined,
  });
});

test("allowlist mode requires a non-empty allowlist", () => {
  const payload = {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "update",
    requestId: "request-empty-allowlist",
    request: {
      channelId: CHANNEL_ID,
      agentName: "Review helper",
      respondTo: "allowlist",
      respondToAllowlist: [],
    },
  };
  assert.equal(parseAgentManagementRequest(payload), null);
});

test("changing away from allowlist mode clears the previous audience", () => {
  const payload = {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "update",
    requestId: "request-clear-allowlist",
    request: {
      channelId: CHANNEL_ID,
      agentName: "Review helper",
      respondTo: "anyone",
    },
  };
  const parsed = parseAgentManagementRequest(payload);
  assert.ok(parsed && parsed.action === "update");
  const updated = updateInputFromRequest(parsed, {
    id: "persona-1",
    displayName: "Review helper",
    systemPrompt: "Review changes.",
    behavior: {
      respondTo: "allowlist",
      respondToAllowlist: ["11".repeat(32)],
      parallelism: 2,
    },
  });
  assert.equal(updated.behavior.respondTo, "anyone");
  assert.deepEqual(updated.behavior.respondToAllowlist, []);
});

test("global config request builds only the narrow requested patch", () => {
  const payload = {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "global_config",
    requestId: "request-global-config",
    intent: "apply",
    request: {
      channelId: CHANNEL_ID,
      effort: "high",
      preferredRuntime: "codex",
    },
  };
  const parsed = parseAgentManagementRequest(payload);
  assert.ok(parsed && parsed.action === "global_config");
  const updated = globalConfigPatchFromRequest(
    parsed,
    "BUZZ_ACP_THINKING_EFFORT",
  );
  assert.deepEqual(updated, {
    preferredRuntime: "codex",
    envVars: { BUZZ_ACP_THINKING_EFFORT: "high" },
  });
});

test("effort updates require a runtime-declared persistence key", () => {
  const update = parseAgentManagementRequest({
    type: AGENT_MANAGEMENT_REQUEST,
    action: "update",
    requestId: "request-agent-effort",
    request: {
      channelId: CHANNEL_ID,
      agentName: "Review helper",
      effort: "high",
    },
  });
  assert.ok(update && update.action === "update");
  assert.throws(
    () =>
      updateInputFromRequest(update, {
        id: "persona-1",
        displayName: "Review helper",
        systemPrompt: "Review changes.",
      }),
    /does not support effort configuration/,
  );
});

test("global config audit claims report actual respawn results", () => {
  assert.equal(
    globalConfigAuditTarget({ restartedCount: 0, failedRestartCount: 0 }),
    "saved global agent defaults; no running agent was respawned",
  );
  assert.equal(
    globalConfigAuditTarget({ restartedCount: 2, failedRestartCount: 1 }),
    "saved global agent defaults; restarted 2 affected running agent(s), 1 restart failure(s)",
  );
});

test("global config rejects empty, unknown, and invalid-effort patches", () => {
  for (const request of [
    { channelId: CHANNEL_ID },
    { channelId: CHANNEL_ID, envVars: {} },
    { channelId: CHANNEL_ID, effort: "turbo" },
    { channelId: CHANNEL_ID, autoApproveAgentDrafts: true },
  ]) {
    assert.equal(
      parseAgentManagementRequest({
        type: AGENT_MANAGEMENT_REQUEST,
        action: "global_config",
        requestId: "request-invalid-global-config",
        request,
      }),
      null,
    );
  }
});

test("global config review rows expose env keys but never values", () => {
  const request = parseAgentManagementRequest({
    type: AGENT_MANAGEMENT_REQUEST,
    action: "global_config",
    requestId: "request-secret-review",
    request: {
      channelId: CHANNEL_ID,
      envVars: {
        OPENAI_API_KEY: "must-not-render",
        OPENAI_HOST: "http://lane",
      },
    },
  });
  assert.ok(request && request.action === "global_config");
  const rows = globalConfigReviewRows(request);
  assert.deepEqual(rows, [["Environment keys", "OPENAI_API_KEY, OPENAI_HOST"]]);
  assert.equal(JSON.stringify(rows).includes("must-not-render"), false);
  assert.equal(JSON.stringify(rows).includes("http://lane"), false);
});

test("rejects invalid optional update text instead of silently dropping it", () => {
  for (const [field, value] of [
    ["displayName", false],
    ["systemPrompt", null],
    ["runtime", 42],
    ["provider", []],
    ["model", "   "],
    ["parallelism", 0],
    ["parallelism", 33],
    ["namePool", "Alpha, Beta"],
    ["namePool", ["Alpha", "   "]],
  ]) {
    const payload = {
      type: AGENT_MANAGEMENT_REQUEST,
      action: "update",
      requestId: "request-invalid-update-text",
      request: {
        channelId: CHANNEL_ID,
        agentName: "Review helper",
        [field]: value,
      },
    };
    assert.equal(parseAgentManagementRequest(payload), null);
  }
});

test("allows agents to update only personal, editable profiles", () => {
  assert.equal(
    requestTargetsEditablePersona({ isBuiltIn: false, sourceTeam: null }),
    true,
  );
  assert.equal(
    requestTargetsEditablePersona({ isBuiltIn: true, sourceTeam: null }),
    true,
  );
  assert.equal(
    requestTargetsEditablePersona({ isBuiltIn: false, sourceTeam: "team" }),
    false,
  );
});
