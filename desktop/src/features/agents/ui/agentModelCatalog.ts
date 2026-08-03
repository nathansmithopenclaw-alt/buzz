import type { PersonaModelOption } from "./agentConfigOptions";
import { getProviderEffortConfig } from "./buzzAgentConfig";

type CatalogModel = PersonaModelOption & { group: string };

const ANTHROPIC_MODELS: readonly CatalogModel[] = [
  { id: "opus", label: "Claude Opus (current)", group: "Opus" },
  { id: "sonnet", label: "Claude Sonnet (current)", group: "Sonnet" },
  { id: "haiku", label: "Claude Haiku (current)", group: "Haiku" },
  { id: "claude-fable-5", label: "Claude Fable 5", group: "Fable" },
  { id: "claude-opus-5", label: "Claude Opus 5", group: "Opus" },
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
];

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
): string | null {
  const trimmed = model.trim();
  const qualifier = trimmed.match(/^(.*)\[([^\]]+)]$/);
  if (!qualifier) return null;
  const [, base, value] = qualifier;
  if (
    runtimeId === "claude" &&
    value.toLowerCase() === "1m" &&
    /(?:opus|sonnet|haiku)/i.test(base)
  ) {
    return null;
  }
  return `Remove [${value}] from the model ID and choose it in Reasoning effort instead. Bracket qualifiers are not accepted by this harness.`;
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
