import type {
  AcpConfigOptionValue,
  ManagedAgentBackend,
} from "@/shared/api/types";
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
 * Map a dropdown selection back to the value persisted via
 * `persistAgentEffortLevel`: the sentinel clears effort (null → adapter
 * default), any other value is the explicit effort level.
 */
export function effortSelectionToPersistedValue(value: string): string | null {
  return value === EFFORT_DEFAULT_DROPDOWN_VALUE ? null : value;
}
