import assert from "node:assert/strict";
import test from "node:test";

import {
  mergeRuntimeModelCatalog,
  validateEffortSelection,
  validateModelSelection,
} from "./agentModelCatalog.ts";

test("Claude catalog unions live rows, curated versions, and current saved value", () => {
  const options = mergeRuntimeModelCatalog({
    currentModel: "team-private-model",
    discovered: [{ id: "opus", label: "Opus" }],
    provider: "",
    runtimeId: "claude",
  });

  assert.equal(
    options.find((option) => option.id === "opus")?.badge,
    "Available now",
  );
  assert.equal(
    options.find((option) => option.id === "claude-opus-4-1-20250805")?.badge,
    "Catalog only",
  );
  assert.equal(
    options.find((option) => option.id === "team-private-model")?.badge,
    "Current value",
  );
  assert.deepEqual(
    ["opus", "sonnet", "haiku"].map(
      (id) => options.find((option) => option.id === id)?.label,
    ),
    ["Opus", "Claude Sonnet (current)", "Claude Haiku (current)"],
  );
  assert.equal(options.length, 17, "16 catalog targets plus one custom value");
  assert.equal(
    options
      .filter((option) => option.id.startsWith("claude-"))
      .every((option) => option.disabled !== true),
    true,
    "Claude curated rows are selectable because save makes them advertised",
  );
});

test("dead effort bracket qualifiers fail closed while Claude 1m stays compatible", () => {
  assert.match(
    validateModelSelection("gpt-5.6-sol[ultra]", "buzz-agent"),
    /Reasoning effort/,
  );
  assert.equal(validateModelSelection("opus[1m]", "claude"), null);
});

test("Claude native advertised effort set is authoritative", () => {
  const native = ["default", "low", "medium", "high", "xhigh", "max"].map(
    (id) => ({ id }),
  );
  assert.equal(
    validateEffortSelection({
      effort: "default",
      model: "claude-opus-4-1-20250805",
      nativeEffortOptions: native,
      provider: "anthropic",
      runtimeId: "claude",
    }),
    null,
  );
  assert.match(
    validateEffortSelection({
      effort: "minimal",
      model: "claude-opus-4-1-20250805",
      nativeEffortOptions: native,
      provider: "anthropic",
      runtimeId: "claude",
    }),
    /not valid/,
  );
});

test("Claude discovery failure falls back to the Anthropic table, not all seven values", () => {
  assert.match(
    validateEffortSelection({
      effort: "none",
      model: "claude-opus-4-6",
      nativeEffortOptions: null,
      provider: "",
      runtimeId: "claude",
    }),
    /not valid/,
  );
  assert.equal(
    validateEffortSelection({
      effort: "high",
      model: "claude-opus-4-6",
      nativeEffortOptions: null,
      provider: "",
      runtimeId: "claude",
    }),
    null,
  );
});
