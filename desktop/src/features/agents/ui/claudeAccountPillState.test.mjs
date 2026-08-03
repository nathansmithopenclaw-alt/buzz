import assert from "node:assert/strict";
import test from "node:test";

import { claudeAccountPillShowsWarning } from "./claudeAccountPillState.ts";

test("deliberate credentials-file use is neutral while errors and shadowing warn", () => {
  assert.equal(claudeAccountPillShowsWarning(null, false), false);
  assert.equal(claudeAccountPillShowsWarning("restart failed", false), true);
  assert.equal(claudeAccountPillShowsWarning(null, true), true);
});
