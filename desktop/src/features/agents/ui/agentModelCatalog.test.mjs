import assert from "node:assert/strict";
import test from "node:test";

import {
  modelWithEffortQualifier,
  parseModelEffortQualifier,
  mergeRuntimeModelCatalog,
  validateEffortSelection,
  validateModelSelection,
} from "./agentModelCatalog.ts";

test("Claude catalog canonicalizes aliases, keeps 1m variants selectable, and orders families", () => {
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
    9,
    "the live alias and canonical id collapse to one Opus 4.5 row",
  );
  assert.equal(
    options.length,
    21,
    "20 catalog rows including 1m variants plus one custom value",
  );
  assert.deepEqual(
    options
      .filter((option) => option.group === "Fable")
      .map((option) => option.id),
    [
      "claude-fable-5-1",
      "claude-fable-5-1[1m]",
      "claude-fable-5",
      "claude-fable-5[1m]",
    ],
    "Fable 5.1 is offered ahead of Fable 5, each with its 1m variant",
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
      "claude-opus-5-5",
      "claude-opus-5",
      "opus[1m]",
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
  assert.equal(
    options.some((option) => /\b\d+\.\d+\.\d+\b/.test(option.label)),
    false,
    "CLI versions belong in the dialog blocker, never model option labels",
  );
});

test("dead effort bracket qualifiers fail closed while Claude 1m stays compatible", () => {
  assert.match(
    validateModelSelection("gpt-5.6-sol[ultra]", "buzz-agent"),
    /Model qualifier/,
  );
  assert.equal(validateModelSelection("opus[1m]", "claude"), null);
  assert.equal(validateModelSelection("claude-fable-5[1m]", "claude"), null);
  assert.equal(validateModelSelection("claude-fable-5-1[1m]", "claude"), null);
});

test("an unchanged persisted model never wedges unrelated edits", () => {
  assert.equal(
    validateModelSelection("legacy-model[quirk]", "claude", {
      enabled: false,
      effortOptions: null,
      persistedModel: "legacy-model[quirk]",
    }),
    null,
  );
  assert.match(
    validateModelSelection("changed-model[quirk]", "claude", {
      enabled: false,
      effortOptions: null,
      persistedModel: "legacy-model[quirk]",
    }),
    /not accepted/,
  );
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

// D-913b7643: null and [] are DIFFERENT facts on the discovery result. null means
// the probe said nothing about effort (the OpenAI, Anthropic, mesh and OpenRouter
// paths all return an empty effort list); [] means it advertised an effort control
// with zero valid values. Collapsing the first into the second made both
// validators reject values they have no basis to reject.
test("an unknown effort catalog does not invalidate a saved Claude effort", () => {
  // null = unknown: fall back to the static provider config, which knows "high".
  assert.equal(
    validateEffortSelection({
      effort: "high",
      model: "claude-opus-4-5",
      nativeEffortOptions: null,
      provider: "anthropic",
      runtimeId: "claude",
    }),
    null,
  );
  // [] = the adapter really advertised zero valid levels: reject.
  assert.match(
    validateEffortSelection({
      effort: "high",
      model: "claude-opus-4-5",
      nativeEffortOptions: [],
      provider: "anthropic",
      runtimeId: "claude",
    }),
    /Reasoning effort high is not valid/,
  );
  // A populated discovered catalog still governs.
  assert.equal(
    validateEffortSelection({
      effort: "high",
      model: "claude-opus-4-5",
      nativeEffortOptions: [{ id: "high" }],
      provider: "anthropic",
      runtimeId: "claude",
    }),
    null,
  );
});

test("an unknown effort catalog does not invalidate a changed model qualifier", () => {
  const qualified = "gpt-5.6-sol[medium]";
  // null = unknown: accept, we have no basis to reject.
  assert.equal(
    validateModelSelection(qualified, "codex", {
      enabled: true,
      effortOptions: null,
      persistedModel: "gpt-5.6-sol[low]",
    }),
    null,
  );
  // [] = advertised zero valid levels: reject.
  assert.match(
    validateModelSelection(qualified, "codex", {
      enabled: true,
      effortOptions: [],
      persistedModel: "gpt-5.6-sol[low]",
    }),
    /Reasoning effort medium is not advertised/,
  );
  // A populated catalog still governs, both ways.
  assert.equal(
    validateModelSelection(qualified, "codex", {
      enabled: true,
      effortOptions: [{ id: "medium" }],
      persistedModel: "gpt-5.6-sol[low]",
    }),
    null,
  );
  assert.match(
    validateModelSelection(qualified, "codex", {
      enabled: true,
      effortOptions: [{ id: "low" }],
      persistedModel: "gpt-5.6-sol[low]",
    }),
    /Reasoning effort medium is not advertised/,
  );
  // The persisted-model guard still short-circuits before any catalog check,
  // which is why an UNCHANGED stored qualifier never reached this bug.
  assert.equal(
    validateModelSelection(qualified, "codex", {
      enabled: true,
      effortOptions: [],
      persistedModel: qualified,
    }),
    null,
  );
});
