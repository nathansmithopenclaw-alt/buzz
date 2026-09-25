import {
  formatAgentModelLabel,
  resolveModelLabel,
} from "./formatAgentModelLabel";
import { modelWithEffortQualifier } from "../ui/agentModelCatalog";
import type { AcpRuntimeCatalogEntry, ManagedAgent } from "@/shared/api/types";

/**
 * The model ID a deployed agent runs. A harness that encodes effort in the
 * model ID (`effortModelQualifier`) still takes the agent's own `effortLevel`
 * over the `[level]` saved in its definition's model, because spawn applies
 * the startup effort after the model switch. Showing the saved qualifier
 * would contradict the level the agent runs.
 */
export function effectiveAgentModelId(
  model: string,
  effortLevel: string | null | undefined,
  effortModelQualifier: boolean,
): string {
  if (!effortModelQualifier || !effortLevel || !model.trim()) return model;
  return modelWithEffortQualifier({
    effort: effortLevel,
    inheritedModel: "",
    model,
  });
}

/** Whether the harness this agent launches encodes effort in its model ID. */
export function agentEffortModelQualifier(
  agent: Pick<ManagedAgent, "agentCommand">,
  runtimes: readonly AcpRuntimeCatalogEntry[] | undefined,
): boolean {
  const command = agent.agentCommand.trim();
  return (
    runtimes?.some(
      (runtime) =>
        runtime.effortModelQualifier &&
        (runtime.command?.trim() === command || runtime.id === command),
    ) ?? false
  );
}

/**
 * Model label for the Agents-page card, shared by the definition-grouped
 * card (`AgentPersonaCard`) and the standalone/legacy card
 * (`StandaloneAgentCard`).
 *
 * A materialized `agent` is authoritative once it exists: its `modelSource`
 * says whether the *effective* config (from `resolve_effective_config` on
 * the backend) came from the global default or from an explicit
 * definition/instance value, so the card never has to re-derive that from
 * raw model bytes. Absent or `"global"` renders the default-model label;
 * anything else renders the agent's own resolved model.
 *
 * With NO materialized instance yet (a definition that has never been
 * started), there is no `modelSource` to read — the definition itself is
 * authoritative, so an explicit `personaModel` must render directly rather
 * than falling through to "inherited" for lack of an instance.
 */
export function resolveAgentCardModelLabel(input: {
  agent:
    | Pick<
        ManagedAgent,
        | "modelSource"
        | "model"
        | "provider"
        | "externalExecution"
        | "effortLevel"
      >
    | undefined;
  personaModel: string | null | undefined;
  /** Inference provider for the persona/agent — threads provider-qualified label lookup. */
  provider?: string | null | undefined;
  defaultModel: string;
  /** The agent's harness encodes effort in its model ID. */
  effortModelQualifier?: boolean;
}): string {
  if (input.agent?.externalExecution) return "Runs in Codex Desktop";
  if (input.agent) {
    const { agent } = input;
    const runs = (model: string) =>
      effectiveAgentModelId(
        model,
        agent.effortLevel,
        input.effortModelQualifier ?? false,
      );
    const isInherited = !agent.modelSource || agent.modelSource === "global";
    if (isInherited || !agent.model?.trim()) {
      return formatDefaultModelLabel(runs(input.defaultModel), agent.provider);
    }
    return formatAgentModelLabel(runs(agent.model), agent.provider);
  }
  return input.personaModel?.trim()
    ? formatAgentModelLabel(input.personaModel, input.provider)
    : formatDefaultModelLabel(input.defaultModel, input.provider);
}

export function formatDefaultModelLabel(
  defaultModel: string,
  provider?: string | null | undefined,
) {
  const model = defaultModel.trim();
  return model
    ? `Default model (${resolveModelLabel(model, undefined, provider)})`
    : "Default model";
}
