import type { PersonaModelOption } from "./agentConfigOptions";
import { getProviderEffortConfig } from "./buzzAgentConfig";

type CatalogModel = PersonaModelOption & { group: string };

const ANTHROPIC_MODELS: readonly CatalogModel[] = [
  { id: "claude-fable-5", label: "Claude Fable 5", group: "Fable" },
  {
    id: "claude-fable-5[1m]",
    label: "Claude Fable 5 (1M context)",
    group: "Fable",
  },
  { id: "claude-opus-5", label: "Claude Opus 5", group: "Opus" },
  { id: "opus[1m]", label: "Claude Opus (1M context)", group: "Opus" },
  { id: "claude-opus-4-8", label: "Claude Opus 4.8", group: "Opus" },
  { id: "claude-opus-4-7", label: "Claude Opus 4.7", group: "Opus" },
  { id: "claude-opus-4-6", label: "Claude Opus 4.6", group: "Opus" },
  {
    id: "claude-opus-4-5-20251101",
    label: "Claude Opus 4.5 (2025-11-01)",
    group: "Opus",
  },
  {
    id: "claude-opus-4-1-20250805",
    label: "Claude Opus 4.1 (2025-08-05)",
    group: "Opus",
  },
  {
    id: "claude-opus-4-20250514",
    label: "Claude Opus 4 (2025-05-14)",
    group: "Opus",
  },
  { id: "claude-sonnet-5", label: "Claude Sonnet 5", group: "Sonnet" },
  {
    id: "sonnet[1m]",
    label: "Claude Sonnet (1M context)",
    group: "Sonnet",
  },
  { id: "claude-sonnet-4-6", label: "Claude Sonnet 4.6", group: "Sonnet" },
  {
    id: "claude-sonnet-4-5-20250929",
    label: "Claude Sonnet 4.5 (2025-09-29)",
    group: "Sonnet",
  },
  {
    id: "claude-sonnet-4-20250514",
    label: "Claude Sonnet 4 (2025-05-14)",
    group: "Sonnet",
  },
  {
    id: "claude-haiku-4-5-20251001",
    label: "Claude Haiku 4.5 (2025-10-01)",
    group: "Haiku",
  },
  { id: "haiku[1m]", label: "Claude Haiku (1M context)", group: "Haiku" },
];

const CLAUDE_CANONICAL_IDS: Readonly<Record<string, string>> = {
  fable: "claude-fable-5",
  opus: "claude-opus-5",
  sonnet: "claude-sonnet-5",
  haiku: "claude-haiku-4-5-20251001",
  "claude-opus-4-5": "claude-opus-4-5-20251101",
  "claude-opus-4-1": "claude-opus-4-1-20250805",
  "claude-opus-4": "claude-opus-4-20250514",
  "claude-sonnet-4-5": "claude-sonnet-4-5-20250929",
  "claude-sonnet-4": "claude-sonnet-4-20250514",
  "claude-haiku-4-5": "claude-haiku-4-5-20251001",
};

const CLAUDE_FAMILY_ORDER: Readonly<Record<string, number>> = {
  Fable: 0,
  Opus: 1,
  Sonnet: 2,
  Haiku: 3,
  Other: 4,
};

function canonicalClaudeModelId(id: string): string {
  const trimmed = id.trim();
  return CLAUDE_CANONICAL_IDS[trimmed] ?? trimmed;
}

function mergeClaudeModelCatalog(
  currentModel: string,
  discovered: readonly PersonaModelOption[] | null,
): readonly PersonaModelOption[] {
  const liveByCanonicalId = new Map<string, PersonaModelOption>();
  for (const option of discovered ?? []) {
    const canonicalId = canonicalClaudeModelId(option.id);
    if (!liveByCanonicalId.has(canonicalId)) {
      liveByCanonicalId.set(canonicalId, option);
    }
  }

  const merged: PersonaModelOption[] = [];
  const emitted = new Set<string>();
  const defaultEntry = liveByCanonicalId.get("");
  if (defaultEntry) {
    merged.push(defaultEntry);
    emitted.add("");
  }

  for (const option of ANTHROPIC_MODELS) {
    const canonicalId = canonicalClaudeModelId(option.id);
    merged.push({
      ...option,
      badge: liveByCanonicalId.has(canonicalId)
        ? "Available now"
        : "Catalog only",
    });
    emitted.add(canonicalId);
  }

  for (const option of discovered ?? []) {
    const canonicalId = canonicalClaudeModelId(option.id);
    if (emitted.has(canonicalId)) continue;
    merged.push({
      ...option,
      badge: option.id.trim() ? "Available now" : undefined,
      group: option.id.trim() ? modelFamily(option.id) : undefined,
    });
    emitted.add(canonicalId);
  }

  const trimmedCurrent = currentModel.trim();
  const currentCanonicalId = canonicalClaudeModelId(trimmedCurrent);
  if (trimmedCurrent && !emitted.has(currentCanonicalId)) {
    merged.push({
      id: trimmedCurrent,
      label: trimmedCurrent,
      badge: "Current value",
      group: modelFamily(trimmedCurrent),
    });
  }

  const catalogOrder = new Map(
    ANTHROPIC_MODELS.map((option, index) => [option.id, index]),
  );
  return merged.sort((left, right) => {
    if (left.id === "") return -1;
    if (right.id === "") return 1;
    const leftGroup = modelFamily(left.id);
    const rightGroup = modelFamily(right.id);
    const familyDelta =
      (CLAUDE_FAMILY_ORDER[leftGroup] ?? CLAUDE_FAMILY_ORDER.Other) -
      (CLAUDE_FAMILY_ORDER[rightGroup] ?? CLAUDE_FAMILY_ORDER.Other);
    if (familyDelta !== 0) return familyDelta;
    const leftOrder = catalogOrder.get(canonicalClaudeModelId(left.id));
    const rightOrder = catalogOrder.get(canonicalClaudeModelId(right.id));
    if (leftOrder !== undefined || rightOrder !== undefined) {
      return (
        (leftOrder ?? Number.MAX_SAFE_INTEGER) -
        (rightOrder ?? Number.MAX_SAFE_INTEGER)
      );
    }
    return left.label.localeCompare(right.label);
  });
}

const OPENAI_MODELS: readonly CatalogModel[] = [
  { id: "gpt-5.6", label: "GPT-5.6 (Sol alias)", group: "GPT-5.6" },
  { id: "gpt-5.6-sol", label: "GPT-5.6 Sol", group: "GPT-5.6" },
  { id: "gpt-5.6-terra", label: "GPT-5.6 Terra", group: "GPT-5.6" },
  { id: "gpt-5.6-luna", label: "GPT-5.6 Luna", group: "GPT-5.6" },
  { id: "gpt-5.5", label: "GPT-5.5", group: "GPT-5.5" },
  { id: "gpt-5.5-pro", label: "GPT-5.5 Pro", group: "GPT-5.5" },
  { id: "gpt-5.4", label: "GPT-5.4", group: "GPT-5.4" },
  { id: "gpt-5.4-pro", label: "GPT-5.4 Pro", group: "GPT-5.4" },
  { id: "gpt-5.4-mini", label: "GPT-5.4 mini", group: "GPT-5.4" },
  { id: "gpt-5.4-nano", label: "GPT-5.4 nano", group: "GPT-5.4" },
  { id: "gpt-5.3-codex", label: "GPT-5.3 Codex", group: "GPT-5.3" },
  { id: "gpt-5.2", label: "GPT-5.2", group: "GPT-5.2" },
  { id: "gpt-5.2-pro", label: "GPT-5.2 Pro", group: "GPT-5.2" },
  { id: "gpt-5.2-codex", label: "GPT-5.2 Codex", group: "GPT-5.2" },
  { id: "gpt-5.1", label: "GPT-5.1", group: "GPT-5.1" },
  { id: "gpt-5", label: "GPT-5", group: "GPT-5" },
  { id: "gpt-5-pro", label: "GPT-5 Pro", group: "GPT-5" },
  { id: "gpt-5-mini", label: "GPT-5 mini", group: "GPT-5" },
  { id: "gpt-5-nano", label: "GPT-5 nano", group: "GPT-5" },
];

function catalogFor(runtimeId: string, provider: string) {
  if (runtimeId === "claude") return ANTHROPIC_MODELS;
  if (runtimeId === "codex") return OPENAI_MODELS;
  const normalizedProvider = provider.trim().toLowerCase();
  if (normalizedProvider === "anthropic") return ANTHROPIC_MODELS;
  if (normalizedProvider === "openai") return OPENAI_MODELS;
  return [];
}

export function mergeRuntimeModelCatalog({
  currentModel,
  discovered,
  provider,
  runtimeId,
}: {
  currentModel: string;
  discovered: readonly PersonaModelOption[] | null;
  provider: string;
  runtimeId: string;
}): readonly PersonaModelOption[] {
  if (runtimeId === "claude") {
    return mergeClaudeModelCatalog(currentModel, discovered);
  }
  const live = discovered ?? [];
  const liveIds = new Set(live.map((option) => option.id.trim()));
  const merged: PersonaModelOption[] = live.map((option) => ({
    ...option,
    badge: option.id.trim() ? "Available now" : undefined,
    group: option.id.trim() ? modelFamily(option.id) : undefined,
  }));

  for (const option of catalogFor(runtimeId, provider)) {
    if (liveIds.has(option.id)) continue;
    merged.push({
      ...option,
      badge: "Catalog only",
      disabled: runtimeId !== "claude" && option.id !== currentModel.trim(),
    });
  }

  const trimmedCurrent = currentModel.trim();
  if (
    trimmedCurrent &&
    !merged.some((option) => option.id === trimmedCurrent)
  ) {
    merged.push({
      id: trimmedCurrent,
      label: trimmedCurrent,
      badge: "Current value",
      group: modelFamily(trimmedCurrent),
    });
  }

  return merged;
}

function modelFamily(modelId: string): string {
  const id = modelId.toLowerCase();
  if (id.includes("fable")) return "Fable";
  if (id.includes("opus")) return "Opus";
  if (id.includes("sonnet")) return "Sonnet";
  if (id.includes("haiku")) return "Haiku";
  const gpt = id.match(/gpt-(5(?:\.\d+)?)/)?.[1];
  return gpt ? `GPT-${gpt}` : "Other";
}

export function validateModelSelection(
  model: string,
  runtimeId: string,
  qualifierCapability?: {
    enabled: boolean;
    effortOptions: ReadonlyArray<{ id: string }> | null;
    persistedModel?: string | null;
  },
): string | null {
  // An existing stored value must not wedge unrelated edits. Preserve it
  // byte-for-byte unless the user explicitly changes the model field.
  if (
    qualifierCapability?.persistedModel !== undefined &&
    model === qualifierCapability.persistedModel
  ) {
    return null;
  }
  const trimmed = model.trim();
  const qualifier = trimmed.match(/^(.*)\[([^\]]+)]$/);
  if (!qualifier) return null;
  const [, base, value] = qualifier;
  if (
    runtimeId === "claude" &&
    value.toLowerCase() === "1m" &&
    /^(?:claude-|fable(?:$|-)|opus(?:$|-)|sonnet(?:$|-)|haiku(?:$|-))/i.test(
      base.trim(),
    )
  ) {
    return null;
  }
  if (qualifierCapability?.enabled) {
    if (qualifierCapability.effortOptions === null) return null;
    return qualifierCapability.effortOptions.some(
      (option) => option.id === value,
    )
      ? null
      : `Reasoning effort ${value} is not advertised for ${base}.`;
  }
  return `Model qualifier [${value}] is not accepted by this harness. Choose a supported model variant or enter an unqualified model ID.`;
}

export function parseModelEffortQualifier(model: string): {
  baseModel: string;
  effort: string | null;
} {
  const trimmed = model.trim();
  const match = trimmed.match(/^(.*)\[([^\]]+)]$/);
  return match
    ? { baseModel: match[1].trim(), effort: match[2].trim() || null }
    : { baseModel: trimmed, effort: null };
}

export function modelWithEffortQualifier({
  effort,
  inheritedModel,
  model,
}: {
  effort: string;
  inheritedModel: string;
  model: string;
}): string {
  const explicitBase = parseModelEffortQualifier(model).baseModel;
  if (!effort) return explicitBase;
  const inheritedBase = parseModelEffortQualifier(inheritedModel).baseModel;
  const baseModel = explicitBase || inheritedBase;
  return baseModel ? `${baseModel}[${effort}]` : "";
}

export function validateEffortSelection({
  effort,
  model,
  nativeEffortOptions,
  provider,
  runtimeId,
}: {
  effort: string;
  model: string;
  nativeEffortOptions: ReadonlyArray<{ id: string }> | null;
  provider: string;
  runtimeId: string;
}): string | null {
  const trimmed = effort.trim();
  if (!trimmed) return null;
  const valid =
    runtimeId === "claude" && nativeEffortOptions !== null
      ? nativeEffortOptions.map((option) => option.id)
      : getProviderEffortConfig(
          runtimeId === "claude" ? "anthropic" : provider,
          model,
        ).validValues;
  return (valid as readonly string[]).includes(trimmed)
    ? null
    : `Reasoning effort ${trimmed} is not valid for the selected model.`;
}
