import assert from "node:assert/strict";
import test from "node:test";
import { shouldCheckClaudeCli } from "./claudeCliBlocker.ts";

test("only the verified local Claude model activates the CLI blocker", () => {
  assert.equal(shouldCheckClaudeCli("claude", "claude-opus-5-5"), true);
  assert.equal(shouldCheckClaudeCli("claude", "claude-opus-5"), false);
  assert.equal(shouldCheckClaudeCli("codex", "claude-opus-5-5"), false);
});
