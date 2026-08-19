import assert from "node:assert/strict";
import test from "node:test";

import { canonicalizeOverrides } from "./featureAliases.ts";

const FEATURE_IDS = new Set(["remoteControl", "dispatch"]);

test("canonicalizeOverrides maps retired fleetVisibility onto remoteControl", () => {
  assert.deepEqual(
    canonicalizeOverrides({ fleetVisibility: true }, FEATURE_IDS),
    { remoteControl: true },
  );
});

test("canonicalizeOverrides lets remoteControl win over the alias", () => {
  assert.deepEqual(
    canonicalizeOverrides(
      { fleetVisibility: true, remoteControl: false },
      FEATURE_IDS,
    ),
    { remoteControl: false },
  );
});
