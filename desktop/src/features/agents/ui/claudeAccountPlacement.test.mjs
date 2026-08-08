import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const read = (name) => readFile(new URL(name, import.meta.url), "utf8");

test("Claude account selector moves off both agents-grid card implementations", async () => {
  const [editDialog, editField, managedRow, unifiedSection] = await Promise.all(
    [
      read("./AgentInstanceEditDialog.tsx"),
      read("./EditAgentClaudeAccountField.tsx"),
      read("./ManagedAgentRow.tsx"),
      read("./UnifiedAgentsSection.tsx"),
    ],
  );

  assert.match(editDialog, /<EditAgentClaudeAccountField agent=\{agent\} \/>/);
  assert.match(editField, /<ClaudeAccountPill agent=\{agent\} \/>/);
  assert.doesNotMatch(managedRow, /ClaudeAccountPill/);
  assert.doesNotMatch(unifiedSection, /ClaudeAccountPill/);
});

test("account selector does not promise unavailable usage metering", async () => {
  const pill = await read("./ClaudeAccountPill.tsx");

  assert.doesNotMatch(pill, /fiveHour|usageStatus|utilization|resetsAt/);
  assert.match(pill, /Saved account/);
});
