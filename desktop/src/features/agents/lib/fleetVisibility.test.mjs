import assert from "node:assert/strict";
import test from "node:test";

import {
  KIND_HOST_DECLARATION,
  KIND_MANAGED_AGENT,
} from "@/shared/constants/kinds";
import { deriveFleetVisibilityBootstrap } from "./fleetVisibility.ts";

const owner = "a".repeat(64);
const local = "1".repeat(64);
const remote = "2".repeat(64);

function event(kind, d, content = "{}", created_at = 1) {
  return {
    id: `${kind}-${d}`,
    pubkey: owner,
    created_at,
    kind,
    tags: [["d", d]],
    content,
    sig: "",
  };
}

test("detects only owner projections without keyed local records", () => {
  const result = deriveFleetVisibilityBootstrap(
    [event(KIND_MANAGED_AGENT, local), event(KIND_MANAGED_AGENT, remote)],
    [local.toUpperCase()],
  );
  assert.deepEqual(result.unmatchedAgentPubkeys, [remote]);
});

test("uses the newest valid 30180 host name", () => {
  const result = deriveFleetVisibilityBootstrap(
    [
      event(
        KIND_HOST_DECLARATION,
        "host-a",
        JSON.stringify({
          v: 1,
          name: "Mini",
          platform: "macos",
          created_at: 1,
        }),
        1,
      ),
      event(
        KIND_HOST_DECLARATION,
        "host-b",
        JSON.stringify({
          v: 1,
          name: "MacBook",
          platform: "macos",
          created_at: 2,
        }),
        2,
      ),
    ],
    [],
  );
  assert.equal(result.hostName, "MacBook");
});
