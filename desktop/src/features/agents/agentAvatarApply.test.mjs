import assert from "node:assert/strict";
import test from "node:test";

import {
  AGENT_AVATAR_APPLY_REQUEST,
  parseAgentAvatarApplyRequest,
} from "./agentAvatarApply.ts";

test("parses only the owner-authorization envelope", () => {
  const request = {
    type: AGENT_AVATAR_APPLY_REQUEST,
    requestId: "19c7ea88-6149-4596-93ed-cfa9d685fa7b",
    authorizationEvent: '{"id":"signed-event"}',
  };
  assert.deepEqual(parseAgentAvatarApplyRequest(request), request);
  assert.equal(
    parseAgentAvatarApplyRequest({
      ...request,
      url: "https://unsigned.example/a.png",
    }),
    null,
  );
});
