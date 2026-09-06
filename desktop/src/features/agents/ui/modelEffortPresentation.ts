import type { PersonaModelOption } from "./agentConfigOptions";
import { parseModelEffortQualifier } from "./agentModelCatalog";

/** Separate the visible model label from its legacy effort transport. Values
 * remain byte-identical; the runtime capability controls this presentation. */
export function modelEffortPresentation(
  options: readonly PersonaModelOption[],
  current: string,
  enabled: boolean,
): readonly PersonaModelOption[] {
  if (!enabled) return options;
  const { baseModel, effort } = parseModelEffortQualifier(current);
  if (!effort) return options;
  const base = options.find((option) => option.id === baseModel);
  return options
    .filter((option) => option.id !== baseModel)
    .map((option) =>
      option.id === current
        ? {
            ...option,
            label: base?.label ?? baseModel,
            badge: base?.badge ?? option.badge,
          }
        : option,
    );
}
