import assert from "node:assert/strict";
import test from "node:test";

import {
  classifyAgentManagementOrigin,
  dequeueAgentManagementRequest,
  enqueueAgentManagementRequest,
} from "./agentManagementBuffer.ts";

const AGENT = "a".repeat(64);
const CHANNEL = "channel-1";
const OWNED_AGENT = [{ pubkey: AGENT }];
const SHARED_CHANNEL = [
  { id: CHANNEL, isMember: true, memberPubkeys: [AGENT] },
];

test("buffers a draft until ownership and channel data resolve", () => {
  assert.equal(
    classifyAgentManagementOrigin(undefined, SHARED_CHANNEL, AGENT, CHANNEL),
    "buffer",
  );
  assert.equal(
    classifyAgentManagementOrigin(OWNED_AGENT, undefined, AGENT, CHANNEL),
    "buffer",
  );
});

test("accepts an owned agent drafting from a shared channel", () => {
  assert.equal(
    classifyAgentManagementOrigin(OWNED_AGENT, SHARED_CHANNEL, AGENT, CHANNEL),
    "accept",
  );
});

test("rejects a draft when the owner or agent is outside the claimed channel", () => {
  assert.equal(
    classifyAgentManagementOrigin(
      OWNED_AGENT,
      [{ id: CHANNEL, isMember: false, memberPubkeys: [AGENT] }],
      AGENT,
      CHANNEL,
    ),
    "reject",
  );
  assert.equal(
    classifyAgentManagementOrigin(
      OWNED_AGENT,
      [{ id: CHANNEL, isMember: true, memberPubkeys: [] }],
      AGENT,
      CHANNEL,
    ),
    "reject",
  );
});

test("rejects a draft from an agent this Desktop does not own", () => {
  assert.equal(
    classifyAgentManagementOrigin(
      [{ pubkey: "b".repeat(64) }],
      SHARED_CHANNEL,
      AGENT,
      CHANNEL,
    ),
    "reject",
  );
});

test("queues rapid drafts sequentially instead of dropping the second", () => {
  const queue = [];
  const seen = new Set();
  const first = {
    agentPubkey: AGENT,
    request: {
      type: "agent_management_request",
      action: "create",
      requestId: "request-1",
      request: {
        channelId: CHANNEL,
        displayName: "First",
        systemPrompt: "First prompt",
      },
    },
  };
  const second = {
    agentPubkey: AGENT,
    request: {
      ...first.request,
      requestId: "request-2",
      request: {
        ...first.request.request,
        displayName: "Second",
      },
    },
  };

  assert.equal(enqueueAgentManagementRequest(queue, seen, first), true);
  const active = dequeueAgentManagementRequest(queue);
  assert.equal(enqueueAgentManagementRequest(queue, seen, second), true);
  assert.equal(enqueueAgentManagementRequest(queue, seen, second), false);

  assert.equal(active.request.requestId, "request-1");
  assert.equal(
    dequeueAgentManagementRequest(queue).request.requestId,
    "request-2",
  );
  assert.equal(dequeueAgentManagementRequest(queue), undefined);
});
