import type {
  AcpRuntimeCatalogEntry,
  ModelConnectionSupport,
} from "@/shared/api/types";
import type { PersonaDropdownOption } from "@/features/agents/ui/agentConfigOptions";
import type { ModelConnection } from "./api";

/** Existing provider-based connection flows keep their native discovery/save policy. */
export function usesConnectionProvider(
  runtime: AcpRuntimeCatalogEntry | undefined,
) {
  return runtime?.providerOptions?.some(([id]) => id === "buzz-local") === true;
}

const PREFIX = "saved-connection:";

/** UI identity includes the source; only the raw model is persisted to the agent. */
export function connectionModelValue(connectionId: string, model: string) {
  return PREFIX + JSON.stringify([connectionId, model]);
}

export function resolveConnectionModelValue(
  value: string,
  rows: readonly ModelConnection[],
): { connection: ModelConnection; model: string } | null {
  if (!value.startsWith(PREFIX)) return null;
  try {
    const parsed: unknown = JSON.parse(value.slice(PREFIX.length));
    if (
      !Array.isArray(parsed) ||
      parsed.length !== 2 ||
      typeof parsed[0] !== "string" ||
      typeof parsed[1] !== "string"
    )
      return null;
    const [id, model] = parsed;
    const connection = rows.find((row) => row.id === id);
    return connection?.models.includes(model) ? { connection, model } : null;
  } catch {
    return null;
  }
}

export function qualifiedConnectionProtocol(
  row: ModelConnection,
  model: string,
  support: ModelConnectionSupport,
) {
  const q = row.qualification;
  if (q?.revision !== 1 || q.model !== model) return null;
  return (
    q.protocols.find((check) => check.protocol === support.protocol) ?? null
  );
}

export function connectionModelChoices(
  rows: readonly ModelConnection[],
  support: ModelConnectionSupport | null | undefined,
): PersonaDropdownOption[] {
  if (!support) return [];
  return rows.flatMap((row) =>
    row.models.map((model) => {
      const check = qualifiedConnectionProtocol(row, model, support);
      const reason =
        support.setupReason ??
        (!check
          ? "Test this model in Settings → AI"
          : !check.tool_round_trip
            ? (check.reason ?? "Test this connection again in Settings → AI")
            : null);
      return {
        value: connectionModelValue(row.id, model),
        label: model,
        group: row.name,
        disabled: reason !== null,
        badge: reason ?? undefined,
      };
    }),
  );
}

export function connectionEffortLevels(
  row: ModelConnection,
  model: string,
  support: ModelConnectionSupport,
): string[] {
  const check = qualifiedConnectionProtocol(row, model, support);
  return check?.tool_round_trip ? check.effort_levels : [];
}

/** A missing or unqualified source must never be saved as a hosted fallback. */
export function connectionSelectionError(
  value: string | null,
  options: readonly PersonaDropdownOption[],
  loading: boolean,
  loadError: string | null,
  sourceLabel?: string,
): string | null {
  if (!value) return null;
  if (loadError) return loadError;
  if (loading) return "Loading saved model connections…";
  const choice = options.find((row) => row.value === value);
  if (!choice) {
    let source = sourceLabel;
    if (!source) {
      try {
        source = JSON.parse(value.slice(PREFIX.length))[0];
      } catch {
        /* malformed saved selection */
      }
    }
    return `Saved connection “${source || "unknown"}” or its selected model is unavailable for this harness. Choose another model.`;
  }
  return choice.disabled
    ? (choice.badge ?? "Test this connection again in Settings → AI")
    : null;
}

export function connectionEffortError(
  levels: readonly string[],
  value: string,
): string | null {
  if (levels.length === 0) return null;
  return levels.includes(value)
    ? null
    : "Choose a thinking level tested for this connection.";
}

export function connectionAccountReason(
  agent:
    | {
        subscriptionAccountId?: string | null;
        subscriptionAccountInheritDefault?: boolean;
      }
    | undefined,
): string | null {
  return agent?.subscriptionAccountId ||
    agent?.subscriptionAccountInheritDefault
    ? "Remove this agent's subscription account before choosing a saved model connection."
    : null;
}
