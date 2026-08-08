import assert from "node:assert/strict";
import test from "node:test";

import { claudeAccountPillShowsWarning } from "./claudeAccountPillState.ts";

test("only operational errors render warning treatment", () => {
  assert.equal(claudeAccountPillShowsWarning(null), false);
  assert.equal(claudeAccountPillShowsWarning("restart failed"), true);
});
