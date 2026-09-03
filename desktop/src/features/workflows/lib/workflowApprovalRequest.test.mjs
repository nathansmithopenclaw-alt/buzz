import assert from "node:assert/strict";
import test from "node:test";

import { parseWorkflowApprovalRequest } from "./workflowApprovalRequest.ts";

test("parses the targeted approver, opaque reference, and expiration", () => {
  const request = parseWorkflowApprovalRequest([
    ["p", "a".repeat(64)],
    ["d", "f".repeat(64)],
    ["token", "123e4567-e89b-12d3-a456-426614174000"],
    ["expiration", "1785644726"],
  ]);

  assert.equal(request.approverPubkey, "a".repeat(64));
  assert.equal(request.approvalRef, "f".repeat(64));
  assert.equal(request.expiresAt?.toISOString(), "2026-08-02T04:25:26.000Z");
});

test("fails closed when action tags are absent or malformed", () => {
  assert.deepEqual(parseWorkflowApprovalRequest([]), {
    approverPubkey: null,
    expiresAt: null,
    approvalRef: null,
  });
  assert.equal(
    parseWorkflowApprovalRequest([["expiration", "not-a-number"]]).expiresAt,
    null,
  );
  assert.deepEqual(
    parseWorkflowApprovalRequest([
      ["p", "short"],
      ["d", "not-hex"],
      ["token", "not-a-uuid"],
      ["expiration", "9007199254740992"],
    ]),
    {
      approverPubkey: null,
      expiresAt: null,
      approvalRef: null,
    },
  );
});
