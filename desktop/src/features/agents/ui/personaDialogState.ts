import type {
  AgentPersona,
  CreatePersonaInput,
  PersonaBehaviorInput,
  UpdatePersonaInput,
} from "@/shared/api/types";

export type PersonaDialogState = {
  /**
   * Deployed agent whose Agents-page card opened this editor. The dialog edits
   * that agent's thinking level beside the definition, as the profile Edit
   * does. Absent for editors that have no single deployed agent.
   */
  agentPubkey?: string;
  description: string;
  initialValues: CreatePersonaInput | UpdatePersonaInput;
  submitLabel: string;
  title: string;
};

/**
 * Whether the persona dialog's save action should be enabled.
 *
 * A display name is the only required field. The system prompt is optional:
 * core memory is auto-injected at runtime, so a persona need not carry its
 * own prompt. `isPending` blocks double-submits while a save is in flight.
 */
export function canSubmitPersonaDialog(args: {
  displayName: string;
  isPending: boolean;
}): boolean {
  return args.displayName.trim().length > 0 && !args.isPending;
}

export function formatPersonaNamePoolText(namePool: string[] | undefined) {
  return namePool?.join(", ") ?? "";
}

export function parsePersonaNamePoolText(text: string): string[] {
  return text
    .split(",")
    .map((value) => value.trim())
    .filter((value) => value.length > 0);
}

export function createPersonaDialogState(): PersonaDialogState {
  return {
    title: "Create agent",
    description: "Create an agent and start it immediately.",
    submitLabel: "Create agent",
    initialValues: {
      displayName: "",
      avatarUrl: "",
      systemPrompt: "",
      acpCommand: "buzz-acp",
      runtime: undefined,
      model: undefined,
    },
  };
}

/**
 * The deployed agent an Agents-page editor edits beside its definition: the
 * card's agent, and only while that agent still belongs to the edited
 * definition. Editors with no deployed agent resolve to `undefined`.
 */
export function resolvePersonaDialogAgent<
  Agent extends { pubkey: string; personaId?: string | null },
>(
  state: PersonaDialogState | null,
  agents: readonly Agent[],
): Agent | undefined {
  if (!state?.agentPubkey || !("id" in state.initialValues)) return undefined;
  const personaId = state.initialValues.id;
  return agents.find(
    (agent) =>
      agent.pubkey === state.agentPubkey && agent.personaId === personaId,
  );
}

/** Remount key for an editor: its definition and deployed agent, if any. */
export function personaDialogKey(state: PersonaDialogState): string {
  const personaId = "id" in state.initialValues ? state.initialValues.id : "";
  return `${personaId}:${state.agentPubkey ?? ""}`;
}

export function duplicatePersonaDialogState(
  persona: AgentPersona,
): PersonaDialogState {
  return {
    title: `Duplicate ${persona.displayName}`,
    description:
      "Create a new agent by copying this profile and adjusting it as needed.",
    submitLabel: "Create agent",
    initialValues: {
      displayName: `${persona.displayName} copy`,
      avatarUrl: persona.avatarUrl ?? "",
      description: persona.description ?? undefined,
      systemPrompt: persona.systemPrompt,
      acpCommand: persona.acpCommand || "buzz-acp",
      runtime: persona.runtime ?? undefined,
      model: persona.model ?? undefined,
      provider: persona.provider ?? undefined,
      // Carry envVars and namePool into the duplicate. Without this, a
      // duplicated persona that relies on an API key in env_vars would
      // silently fail at spawn until the user re-entered every credential.
      // The user sees the inherited values in the dialog and can clear
      // them if they want a blank template.
      namePool: persona.namePool ?? [],
      envVars: persona.envVars ?? {},
      ...behaviorEntry(persona),
    },
  };
}

/**
 * Seed a dialog behavior group from a stored persona. A quad-less persona
 * yields no `behavior` key at all, keeping initialValues byte-identical to
 * the pre-quad shape (spread-in entry, matching the namePool import pattern).
 */
function behaviorEntry(
  persona: AgentPersona,
): { behavior: PersonaBehaviorInput } | Record<string, never> {
  if (
    persona.respondTo == null &&
    persona.parallelism == null &&
    (persona.sessionPolicy ?? "channel") === "channel"
  ) {
    return {};
  }
  return {
    behavior: {
      respondTo: persona.respondTo ?? undefined,
      respondToAllowlist:
        persona.respondTo === "allowlist"
          ? persona.respondToAllowlist
          : undefined,
      parallelism: persona.parallelism ?? undefined,
      sessionPolicy: persona.sessionPolicy ?? "channel",
    },
  };
}

export function editPersonaDialogState(
  persona: AgentPersona,
  accessSource?: Pick<AgentPersona, "respondTo" | "respondToAllowlist">,
): PersonaDialogState {
  const behaviorSource = accessSource
    ? {
        ...persona,
        respondTo: accessSource.respondTo,
        respondToAllowlist: accessSource.respondToAllowlist,
      }
    : persona;
  return {
    title: "Edit agent",
    description: "",
    submitLabel: "Save changes",
    initialValues: {
      id: persona.id,
      displayName: persona.displayName,
      avatarUrl: persona.avatarUrl ?? "",
      description: persona.description ?? undefined,
      systemPrompt: persona.systemPrompt,
      acpCommand: persona.acpCommand || "buzz-acp",
      runtime: persona.runtime ?? undefined,
      model: persona.model ?? undefined,
      provider: persona.provider ?? undefined,
      // Seed both namePool and envVars from the loaded persona so editing
      // unrelated fields doesn't submit an empty value that wipes them.
      // (Persona update treats Some(empty) as "clear all" intentionally;
      // the dialog must therefore round-trip the existing values.)
      namePool: persona.namePool ?? [],
      envVars: persona.envVars ?? {},
      ...behaviorEntry(behaviorSource),
    },
  };
}
