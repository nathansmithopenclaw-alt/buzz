import assert from "node:assert/strict";
import test from "node:test";

import {
  fleetAgentStatusPresentation,
  hostFleetAgentStatus,
} from "./fleetAgentStatus.ts";

test("Host status uses process truth and promotes a stopped agent error", () => {
  assert.deepEqual(
    hostFleetAgentStatus({
      lastError: null,
      lastErrorCode: null,
      status: "running",
    }),
    { source: "host", state: "running" },
  );
  assert.deepEqual(
    hostFleetAgentStatus({
      lastError: null,
      lastErrorCode: null,
      status: "stopped",
    }),
    { source: "host", state: "stopped" },
  );
  assert.deepEqual(
    hostFleetAgentStatus({
      lastError: "harness exited with status code 1",
      lastErrorCode: null,
      status: "stopped",
    }),
    { source: "host", state: "error" },
  );
});

test("Observer status presentation owns exact vocabulary and timestamp absence", () => {
  assert.deepEqual(
    fleetAgentStatusPresentation({
      source: "observer",
      state: "live",
      lastSeen: 1_000,
      now: 1_090,
    }),
    { label: "live", stale: false, variant: "success" },
  );
  assert.deepEqual(
    fleetAgentStatusPresentation({
      source: "observer",
      state: "quiet",
      lastSeen: null,
      now: 1_090,
    }),
    {
      label: "quiet · last seen not reported",
      stale: false,
      variant: "secondary",
    },
  );
  assert.deepEqual(
    fleetAgentStatusPresentation({
      source: "observer",
      state: "host-unreachable",
      lastSeen: null,
      now: 1_091,
    }),
    { label: "host unreachable", stale: true, variant: "secondary" },
  );
});

test("90s is fresh vocabulary and 91s is stale vocabulary", () => {
  assert.deepEqual(
    fleetAgentStatusPresentation({
      source: "observer",
      state: "live",
      lastSeen: 1_000,
      now: 1_090,
    }),
    { label: "live", stale: false, variant: "success" },
  );
  assert.deepEqual(
    fleetAgentStatusPresentation({
      source: "observer",
      state: "host-unreachable",
      lastSeen: 1_000,
      now: 1_091,
    }),
    {
      label: "host unreachable · last seen 1m ago",
      stale: true,
      variant: "secondary",
    },
  );
});
