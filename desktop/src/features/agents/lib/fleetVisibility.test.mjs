import assert from "node:assert/strict";
import test from "node:test";

import {
  KIND_HOST_DECLARATION,
  KIND_MANAGED_AGENT,
} from "@/shared/constants/kinds";
import {
  classifyFleetVisibilityBootstrapRead,
  deriveFleetVisibilityBootstrap,
  shouldBlockPersonaCreation,
} from "./fleetVisibility.ts";

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

test("excludes this device's own 30180 from the remote host name", () => {
  const result = deriveFleetVisibilityBootstrap(
    [
      event(
        KIND_HOST_DECLARATION,
        "local-host",
        JSON.stringify({
          v: 1,
          name: "This Mac",
          platform: "macos",
          created_at: 2,
        }),
        2,
      ),
      event(
        KIND_HOST_DECLARATION,
        "remote-host",
        JSON.stringify({
          v: 1,
          name: "Remote Mini",
          platform: "macos",
          created_at: 1,
        }),
        1,
      ),
    ],
    [],
    "local-host",
  );
  assert.equal(result.hostName, "Remote Mini");
});

test("auto-selects Host only for an authenticated empty fleet read", () => {
  assert.deepEqual(
    classifyFleetVisibilityBootstrapRead("connected", "connected", [], []),
    { kind: "auto-host" },
  );
});

test("fails closed when first-run detection starts before relay readiness", () => {
  const result = classifyFleetVisibilityBootstrapRead(
    "connecting",
    "connected",
    [],
    [],
  );
  assert.equal(result.kind, "needs-choice");
  assert.equal(result.detectionFailed, true);
});

test("fails closed when relay readiness is lost during first-run detection", () => {
  const result = classifyFleetVisibilityBootstrapRead(
    "connected",
    "disconnected",
    [],
    [],
  );
  assert.equal(result.kind, "needs-choice");
  assert.equal(result.detectionFailed, true);
});

test("fails closed when first-run relay detection errors", () => {
  const result = classifyFleetVisibilityBootstrapRead(
    "connected",
    "connected",
    null,
    [],
  );
  assert.equal(result.kind, "needs-choice");
  assert.equal(result.detectionFailed, true);
});

test("shows the chooser for every nonempty fleet read", () => {
  const result = classifyFleetVisibilityBootstrapRead(
    "connected",
    "connected",
    [event(KIND_MANAGED_AGENT, local)],
    [local],
  );
  assert.equal(result.kind, "needs-choice");
  assert.equal(result.detectionFailed, false);
  assert.equal(result.fleetRecordCount, 1);
});

test("persona creation gating fails open until enabled settings are known", () => {
  assert.equal(shouldBlockPersonaCreation(true, null), false);
  assert.equal(
    shouldBlockPersonaCreation(true, {
      enabled: false,
      role: "observer",
      needsChoice: true,
      hostId: null,
      hostName: null,
    }),
    false,
  );
  assert.equal(
    shouldBlockPersonaCreation(true, {
      enabled: true,
      role: "observer",
      needsChoice: false,
      hostId: null,
      hostName: null,
    }),
    true,
  );
});
