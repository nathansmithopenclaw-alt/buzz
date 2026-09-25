import type { ModelConnection } from "@/features/model-connections/api";
import type { CleanupCatalog, DictationCleanupSource } from "../api";

export const RECOMMENDED_CONNECTION_MODEL = "google/gemini-3.8-flash";
export const cleanupValue = (source: DictationCleanupSource) =>
  JSON.stringify(source);
type Choice = { value: string; label: string; group?: string };

/** Group labels never change the persisted cleanup source. */
export function cleanupOptions(
  catalog: CleanupCatalog | undefined,
  connections: ModelConnection[],
  claudeModels: { id: string; name?: string | null }[],
  current: DictationCleanupSource,
): Choice[] {
  const options: Choice[] = [
    { value: cleanupValue({ kind: "off" }), label: "Off (as spoken)" },
  ];
  const add = (source: DictationCleanupSource, label: string, group: string) =>
    options.push({ value: cleanupValue(source), label, group });
  if (catalog?.agentOpenRouter || current.kind === "agent_open_router") {
    const models = new Map(
      (catalog?.openRouter ?? []).map((model) => [
        model.id,
        model.name || model.id,
      ]),
    );
    if (current.kind === "agent_open_router")
      models.set(current.model, models.get(current.model) ?? current.model);
    for (const [id, label] of models)
      add(
        { kind: "agent_open_router", model: id },
        label + (id === RECOMMENDED_CONNECTION_MODEL ? " · Recommended" : ""),
        "OpenRouter",
      );
    options.push({
      value: `custom:${cleanupValue({ kind: "agent_open_router", model: "" })}`,
      label: "Other model…",
      group: "OpenRouter",
    });
  }
  for (const group of ["Model connections", "Local models"]) {
    for (const connection of connections) {
      const local =
        catalog?.localConnectionIds.includes(connection.id) ?? false;
      if ((group === "Local models") !== local) continue;
      const models = new Set(connection.models);
      if (
        current.kind === "connection" &&
        current.connectionId === connection.id
      )
        models.add(current.model);
      for (const model of models)
        add(
          { kind: "connection", connectionId: connection.id, model },
          `${connection.name} · ${model}${model === RECOMMENDED_CONNECTION_MODEL ? " · Recommended" : ""}`,
          group,
        );
      options.push({
        value: `custom:${cleanupValue({ kind: "connection", connectionId: connection.id, model: "" })}`,
        label: `${connection.name} · Other model…`,
        group,
      });
    }
  }
  const accountId = current.kind === "claude_plan" ? current.accountId : null;
  const models = new Map(
    claudeModels
      .filter((model) => model.id && model.id !== "default")
      .map((model) => [model.id, model.name || model.id]),
  );
  if (current.kind === "claude_plan")
    models.set(current.model, models.get(current.model) ?? current.model);
  for (const [model, label] of models)
    add({ kind: "claude_plan", accountId, model }, label, "Claude plan");
  options.push({
    value: `custom:${cleanupValue({ kind: "claude_plan", accountId, model: "" })}`,
    label: "Other model…",
    group: "Claude plan",
  });
  const value = cleanupValue(current);
  if (!options.some((option) => option.value === value))
    options.push({
      value,
      label:
        current.kind === "off"
          ? "Off (as spoken)"
          : `${current.model} (saved source unavailable)`,
      group: "Saved selection",
    });
  return options;
}
