import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const focusedViews = readFileSync(
  new URL("./UserProfilePanelFocusedViews.tsx", import.meta.url),
  "utf8",
);
const blocker = readFileSync(
  new URL("../../agents/ui/ClaudeCliVersionBlocker.tsx", import.meta.url),
  "utf8",
);

test("profile diagnostics places Claude CLI recovery beside the coded last error", () => {
  assert.match(
    focusedViews,
    /lastErrorCode\s*===\s*CLAUDE_CLI_TOO_OLD_ERROR_CODE/,
  );
  assert.match(focusedViews, /<ClaudeCliVersionBlocker/);
  assert.match(focusedViews, /reportedError=\{managedAgent\.lastError\}/);
  assert.match(focusedViews, /showMessage=\{false\}/);
});

test("reported blocker state keeps the shared update control visible until recovery succeeds", () => {
  assert.match(blocker, /reportedError\s*&&\s*!reportedResolved/);
  assert.match(blocker, /setReportedResolved\(next === null\)/);
  assert.match(blocker, /"Update Claude Code"/);
});
