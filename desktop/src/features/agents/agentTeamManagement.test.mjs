import assert from "node:assert/strict";
import test from "node:test";

import {
  AGENT_TEAM_MANAGEMENT_REQUEST,
  parseAgentTeamManagementRequest,
} from "./agentTeamManagement.ts";

const CHANNEL_ID = "7c07e659-3610-42f4-9a5e-1e9973c09da9";

function createRequest(overrides = {}) {
  return {
    type: AGENT_TEAM_MANAGEMENT_REQUEST,
    action: "create",
    requestId: "70fc630c-a0ca-42e8-9392-326c5d4ef9c1",
    intent: "apply",
    request: {
      channelId: CHANNEL_ID,
      name: "Review team",
      description: "Independent review",
      instructions: "Review from a fresh frame.",
      members: ["Bolt", "Helm"],
    },
    ...overrides,
  };
}

test("parses the strict no-secret team create contract", () => {
  const request = createRequest();
  assert.deepEqual(parseAgentTeamManagementRequest(request), request);
});

test("parses CLI create payloads with omitted optionals", () => {
  const request = createRequest();
  delete request.request.description;
  delete request.request.instructions;
  assert.deepEqual(parseAgentTeamManagementRequest(request), request);
});

test("normalizes legacy null optionals to absent", () => {
  const create = createRequest();
  create.request.description = null;
  create.request.instructions = null;
  assert.deepEqual(parseAgentTeamManagementRequest(create), {
    ...create,
    request: {
      channelId: create.request.channelId,
      name: create.request.name,
      members: create.request.members,
    },
  });

  const update = {
    type: AGENT_TEAM_MANAGEMENT_REQUEST,
    action: "update",
    requestId: "2d9c20e2-ab9b-4858-a65c-75cf6c84f891",
    intent: "apply",
    request: {
      channelId: CHANNEL_ID,
      team: "Review team",
      name: null,
      description: null,
      instructions: null,
      members: null,
    },
  };
  assert.deepEqual(parseAgentTeamManagementRequest(update), {
    ...update,
    request: { channelId: CHANNEL_ID, team: "Review team" },
  });
});

test("requires apply intent and rejects extra authority or secret fields", () => {
  assert.equal(
    parseAgentTeamManagementRequest(createRequest({ intent: "draft" })),
    null,
  );
  const request = createRequest();
  request.request.privateKey = "must-not-cross-the-boundary";
  assert.equal(parseAgentTeamManagementRequest(request), null);
});

test("preserves omitted update members and accepts an explicit empty replacement", () => {
  const base = {
    type: AGENT_TEAM_MANAGEMENT_REQUEST,
    action: "update",
    requestId: "2d9c20e2-ab9b-4858-a65c-75cf6c84f891",
    intent: "apply",
    request: { channelId: CHANNEL_ID, team: "Review team", name: "Reviewers" },
  };
  assert.deepEqual(parseAgentTeamManagementRequest(base), base);
  const clear = { ...base, request: { ...base.request, members: [] } };
  assert.deepEqual(parseAgentTeamManagementRequest(clear), clear);
});

test("accepts only the exact add-to-channel shape", () => {
  const request = {
    type: AGENT_TEAM_MANAGEMENT_REQUEST,
    action: "add_to_channel",
    requestId: "5e62f0ef-56d9-4de7-8c31-e77a269ef5ba",
    intent: "apply",
    request: { channelId: CHANNEL_ID, team: "Review team" },
  };
  assert.deepEqual(parseAgentTeamManagementRequest(request), request);
  assert.equal(
    parseAgentTeamManagementRequest({
      ...request,
      request: { ...request.request, deleteAfterDeploy: true },
    }),
    null,
  );
});
