import assert from "node:assert/strict";
import test from "node:test";

import {
  modelWithEffortQualifier,
  parseModelEffortQualifier,
  mergeRuntimeModelCatalog,
  validateEffortSelection,
  validateModelSelection,
} from "./agentModelCatalog.ts";

test("Claude catalog canonicalizes aliases, adds Opus 5, and orders families", () => {
  const options = mergeRuntimeModelCatalog({
    currentModel: "team-private-model",
    discovered: [
      { id: "claude-sonnet-4-5", label: "Sonnet 4.5 alias" },
      { id: "opus", label: "Opus alias" },
      { id: "claude-opus-4-5-20251101", label: "Opus 4.5 canonical" },
    ],
    provider: "",
    runtimeId: "claude",
  });

  assert.equal(
    options.find((option) => option.id === "claude-opus-5")?.badge,
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
  assert.equal(
    options.filter((option) => option.group === "Opus").length,
    7,
    "the live alias and canonical id collapse to one Opus 4.5 row",
  );
  assert.equal(
    options.length,
    14,
    "13 canonical catalog rows plus one custom value",
  );
  assert.deepEqual(
    [...new Set(options.map((option) => option.group).filter(Boolean))],
    ["Fable", "Opus", "Sonnet", "Haiku", "Other"],
  );
  assert.deepEqual(
    options
      .filter((option) => option.group === "Opus")
      .map((option) => option.id),
    [
      "claude-opus-5",
      "claude-opus-4-8",
      "claude-opus-4-7",
      "claude-opus-4-6",
      "claude-opus-4-5-20251101",
      "claude-opus-4-1-20250805",
      "claude-opus-4-20250514",
    ],
  );
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

test("Codex model qualifiers validate against live adapter effort options", () => {
  const capability = {
    enabled: true,
    effortOptions: ["low", "medium", "high", "xhigh", "max", "ultra"].map(
      (id) => ({ id }),
    ),
  };
  assert.equal(
    validateModelSelection("gpt-5.6-sol[ultra]", "codex", capability),
    null,
  );
  assert.match(
    validateModelSelection("gpt-5.6-sol[impossible]", "codex", capability),
    /not advertised/,
  );
});

test("model qualifier read-back and writes stay per-agent", () => {
  assert.deepEqual(parseModelEffortQualifier("gpt-5.6-sol[xhigh]"), {
    baseModel: "gpt-5.6-sol",
    effort: "xhigh",
  });
  assert.equal(
    modelWithEffortQualifier({
      effort: "ultra",
      inheritedModel: "gpt-5.6-sol[medium]",
      model: "gpt-5.6-sol[xhigh]",
    }),
    "gpt-5.6-sol[ultra]",
  );
  assert.equal(
    modelWithEffortQualifier({
      effort: "high",
      inheritedModel: "gpt-5.6-terra[medium]",
      model: "",
    }),
    "gpt-5.6-terra[high]",
  );
  assert.equal(
    modelWithEffortQualifier({
      effort: "",
      inheritedModel: "gpt-5.6-sol[medium]",
      model: "gpt-5.6-sol[ultra]",
    }),
    "gpt-5.6-sol",
  );
  assert.equal(
    modelWithEffortQualifier({
      effort: "",
      inheritedModel: "gpt-5.6-sol[medium]",
      model: "",
    }),
    "",
  );
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
