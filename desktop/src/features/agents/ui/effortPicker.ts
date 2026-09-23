import type {
  AcpConfigOptionValue,
  ManagedAgentBackend,
} from "@/shared/api/types";
import type { AgentRunLocation } from "../lib/agentAccessWarning";
import type { AgentConfigFieldModel } from "../lib/agentConfigCore";
import type { PersonaDropdownOption } from "./agentConfigOptions";

/**
 * Sentinel dropdown value for "no explicit effort" — reverts the agent to the
 * adapter default at the next spawn. Distinct from any adapter option value.
 */
export const EFFORT_DEFAULT_DROPDOWN_VALUE = "__effort_default__";

/** A new pick or model change must be supported; unavailable discovery never
 * rewrites or rejects an unchanged saved value during an unrelated edit. */
export function validateSavedEffort(
  value: string | null | undefined,
  edited: boolean,
  options: readonly { id: string }[] | null,
): string | null {
  if (
    !value ||
    !edited ||
    options === null ||
    options.some((option) => option.id === value)
  )
    return null;
  return `Thinking level ${value} is not supported by this model. Choose an available level or inherit defaults.`;
}

/**
 * The discovered effort catalog, or null when discovery advertises no effort
 * control at all.
 *
 * `discover_agent_models` returns an EMPTY option list for every runtime that
 * exposes no `thought_level` (the mesh and OpenRouter paths return
 * `effort_options: Vec::new()` outright), so an empty list means "this probe
 * knows nothing about effort" — never "no level is valid". The picker itself
 * falls back to the session config surface in that case, so validating against
 * the empty probe list would reject every level the user can see offered and
 * wedge Save with no reachable selection.
 */
export function discoveredEffortCatalog<T extends { id: string }>(
  effortConfigId: string | null | undefined,
  effortOptions: readonly T[] | null | undefined,
): readonly T[] | null {
  return effortConfigId ? (effortOptions ?? null) : null;
}

/** Gate and options for the Save-gated local effort control. */
export function effortPickerState({
  backend,
  effortConfigId,
  effortOptions,
  currentEffort,
}: {
  backend: ManagedAgentBackend;
  effortConfigId: string | undefined;
  effortOptions: readonly AcpConfigOptionValue[] | undefined;
  currentEffort: string | null;
}): {
  visible: boolean;
  options: PersonaDropdownOption[];
  selectValue: string;
} {
  const visible = backend.type === "local" && effortConfigId !== undefined;

  const options: PersonaDropdownOption[] = [
    { label: "Inherit defaults", value: EFFORT_DEFAULT_DROPDOWN_VALUE },
    ...(effortOptions ?? []).map((option) => ({
      label: option.displayName ?? option.value,
      value: option.value,
    })),
  ];

  // Keep a saved value visible when discovery no longer advertises it.
  // A catalog miss must not masquerade as clearing the explicit override.
  const trimmed = currentEffort?.trim() ?? "";
  if (
    trimmed &&
    !(effortOptions ?? []).some((option) => option.value === trimmed)
  ) {
    options.push({
      label: `${trimmed} (unavailable)`,
      value: trimmed,
      disabled: true,
    });
  }
  const selectValue = trimmed || EFFORT_DEFAULT_DROPDOWN_VALUE;

  return { visible, options, selectValue };
}

/**
 * What model discovery established about the selected model's effort control.
 * The option list alone cannot say this: `discoveredEffortCatalog` returns
 * `null` while a probe is in flight, after it fails, and when the model has no
 * effort control at all.
 */
export type EffortDiscoveryState =
  | "pending"
  | "unavailable"
  | "none"
  | "available";

export function effortDiscoveryState({
  pending,
  discovered,
  effortOptions,
}: {
  pending: boolean;
  /** Discovery returned a response for the current selection. */
  discovered: boolean;
  effortOptions: readonly unknown[] | null;
}): EffortDiscoveryState {
  if (pending) return "pending";
  if (!discovered) return "unavailable";
  return effortOptions !== null && effortOptions.length > 0
    ? "available"
    : "none";
}

/**
 * Read-only copy for a dialog surface that renders no effort control.
 *
 * A deferred harness-native descriptor (Claude Code) is not an unsupported
 * harness: the harness has effort, but only a deployed local agent's
 * Save-gated picker writes it, and only once the selected model's live levels
 * are known. Its copy names the actual state and never claims the harness
 * lacks the setting.
 */
export function unrenderedEffortExplanation(
  fieldModel: AgentConfigFieldModel,
  effortDiscovery: EffortDiscoveryState,
  runLocation: AgentRunLocation | null,
): string {
  const deferred = fieldModel.fields.some(
    (field) =>
      field.kind === "effort" &&
      field.render === "deferredUntilNativeOptionsAvailable",
  );
  if (deferred) {
    // Remote effort is deployment policy; the local write is rejected.
    if (runLocation === "remote") {
      return "Buzz can’t change the thinking level of an agent that runs on a server.";
    }
    switch (effortDiscovery) {
      case "pending":
        return "Checking which thinking levels the selected model offers…";
      case "unavailable":
        return "Buzz couldn’t load thinking levels for the selected model.";
      case "none":
        return "The selected model doesn’t offer thinking levels.";
      case "available":
        return "Thinking level is set on each deployed agent. Open the agent’s profile and choose Edit to change it.";
    }
  }
  const omission = fieldModel.omissions.find((item) => item.kind === "effort");
  return omission?.reason === "ownedByModelId"
    ? "Codex owns reasoning effort in ~/.codex/config.toml (model_reasoning_effort). Buzz leaves that harness-level setting unchanged."
    : "This harness does not advertise or expose a reasoning-effort setting.";
}

/**
 * Map a dropdown selection back to the persisted value sent as
 * `effortLevel` in the locked update payload: the sentinel clears effort
 * (null → adapter default), any other value is the explicit effort level.
 */
export function effortSelectionToPersistedValue(value: string): string | null {
  return value === EFFORT_DEFAULT_DROPDOWN_VALUE ? null : value;
}
