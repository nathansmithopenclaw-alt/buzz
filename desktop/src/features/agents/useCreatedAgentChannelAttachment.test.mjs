import assert from "node:assert/strict";
import test from "node:test";

import { shouldPresentCreatedAgent } from "./useCreatedAgentChannelAttachment.ts";

test("successful auto-approved creation stays silent", () => {
  assert.equal(shouldPresentCreatedAgent(false, false), false);
});

test("manual creation and attachment failures remain visible", () => {
  assert.equal(shouldPresentCreatedAgent(true, false), true);
  assert.equal(shouldPresentCreatedAgent(false, true), true);
});
