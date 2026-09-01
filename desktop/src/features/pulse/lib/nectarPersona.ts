/**
 * Upstream starter reporter for the Summary tab.
 *
 * Not a built-in Welcome teammate (Fizz / Honey / Pollen stay the install
 * trio). Summary setup creates Nectar; the user only picks harness + model.
 */
import { buildInstanceInputForDefinition } from "@/features/agents/lib/instanceInputForDefinition";
import type {
  AcpRuntime,
  AgentPersona,
  CreateManagedAgentInput,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";

export const NECTAR_DISPLAY_NAME = "Nectar";
/** Stable persona id, same shape as `builtin:fizz` — not a display-name match. */
export const NECTAR_PERSONA_ID = "nectar";
export const NECTAR_AVATAR_PATH = "/onboarding/starter-team/nectar.png";
export const NECTAR_DESCRIPTION = "Concentrated daily digest of your rooms.";

/** Same voice as Fizz / Honey / Pollen: job first, then a little bee charm. */
export const NECTAR_SYSTEM_PROMPT =
  "You are Nectar, a concentrated daily digest. When your scheduled Summary run fires, or when someone asks, write a well-formatted easy-language report of what happened in the rooms you can read since the last watermark. Lead with what needs attention, then what moved, then what is parked. Pair the human markdown with a structured JSON status object other agents can query. Be calm, candid, and concise. Add occasional bee wordplay or 🐝 — keep it charming, never distracting. Do not invent work, assign tasks, or write to trackers. Summarize deltas only.";

export function isNectarPersonaId(id: string | null | undefined): boolean {
  return id === NECTAR_PERSONA_ID;
}

export function findNectarAgent<T extends { personaId?: string | null }>(
  agents: readonly T[],
): T | undefined {
  return agents.find((agent) => isNectarPersonaId(agent.personaId));
}

export function findNectarPersona<T extends { id: string }>(
  personas: readonly T[],
): T | undefined {
  return personas.find((persona) => isNectarPersonaId(persona.id));
}

export function nectarCreateInput(input: {
  avatarUrl: string;
  runtime: string;
  model?: string | null;
  provider?: string | null;
}): CreatePersonaInput {
  return {
    personaId: NECTAR_PERSONA_ID,
    displayName: NECTAR_DISPLAY_NAME,
    avatarUrl: input.avatarUrl,
    systemPrompt: NECTAR_SYSTEM_PROMPT,
    runtime: input.runtime,
    model: input.model?.trim() || undefined,
    provider: input.provider?.trim() || undefined,
    namePool: [NECTAR_DISPLAY_NAME],
  };
}

function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

export async function readNectarAvatarDataUrl(
  fetchImpl: typeof fetch = fetch,
): Promise<string> {
  const response = await fetchImpl(NECTAR_AVATAR_PATH);
  if (!response.ok) {
    throw new Error("Could not load Nectar's avatar.");
  }
  const blob = await response.blob();
  const bytes = new Uint8Array(await blob.arrayBuffer());
  const mime = blob.type || "image/png";
  return `data:${mime};base64,${bytesToBase64(bytes)}`;
}

export type EnsureNectarReporterInput = {
  agents: readonly {
    name: string;
    pubkey: string;
    personaId?: string | null;
  }[];
  personas: readonly AgentPersona[];
  runtime: AcpRuntime;
  model?: string | null;
  provider?: string | null;
  avatarDataUrl: string;
  createPersona: (input: CreatePersonaInput) => Promise<AgentPersona>;
  updatePersona?: (input: UpdatePersonaInput) => Promise<AgentPersona>;
  createManagedAgent: (
    input: CreateManagedAgentInput,
  ) => Promise<{ agent: { pubkey: string } }>;
  toInstanceInput?: (
    persona: AgentPersona,
    runtime: AcpRuntime,
  ) => Promise<CreateManagedAgentInput>;
};

export async function ensureNectarReporter(
  input: EnsureNectarReporterInput,
): Promise<string> {
  const existingAgent = findNectarAgent(input.agents);
  if (existingAgent) return existingAgent.pubkey;

  const runtimeId = input.runtime.id;
  const model = input.model?.trim() || null;
  const provider = input.provider?.trim() || null;
  let persona = findNectarPersona(input.personas) ?? null;

  if (!persona) {
    persona = await input.createPersona(
      nectarCreateInput({
        avatarUrl: input.avatarDataUrl,
        runtime: runtimeId,
        model,
        provider,
      }),
    );
  } else if (
    input.updatePersona &&
    (persona.runtime !== runtimeId ||
      (model && persona.model !== model) ||
      (provider && persona.provider !== provider) ||
      !persona.avatarUrl)
  ) {
    persona = await input.updatePersona({
      id: persona.id,
      displayName: NECTAR_DISPLAY_NAME,
      avatarUrl: persona.avatarUrl ?? input.avatarDataUrl,
      systemPrompt: persona.systemPrompt || NECTAR_SYSTEM_PROMPT,
      runtime: runtimeId,
      model: model ?? persona.model ?? undefined,
      provider: provider ?? persona.provider ?? undefined,
      namePool:
        persona.namePool.length > 0 ? persona.namePool : [NECTAR_DISPLAY_NAME],
    });
  }

  const toInstanceInput =
    input.toInstanceInput ??
    ((nextPersona, runtime) =>
      buildInstanceInputForDefinition(nextPersona, runtime));
  const created = await input.createManagedAgent(
    await toInstanceInput(persona, input.runtime),
  );
  return created.agent.pubkey;
}
