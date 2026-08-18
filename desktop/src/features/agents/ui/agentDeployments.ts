import type { ManagedAgent } from "@/shared/api/types";

export type AgentDeployment = Pick<ManagedAgent, "name" | "pubkey">;

function compactPubkey(pubkey: string) {
  const value = pubkey.trim();
  if (value.length <= 12) return value || "unknown key";
  return `${value.slice(0, 6)}…${value.slice(-6)}`;
}

export function indexAgentDeployments(
  agents: readonly ManagedAgent[],
): ReadonlyMap<string, AgentDeployment[]> {
  const deployments = new Map<string, AgentDeployment[]>();

  for (const agent of agents) {
    if (!agent.personaId) continue;
    const linked = deployments.get(agent.personaId) ?? [];
    linked.push({ name: agent.name, pubkey: agent.pubkey });
    deployments.set(agent.personaId, linked);
  }

  return deployments;
}

export function buildAgentMemorySources(
  deployments: readonly AgentDeployment[],
): AgentDeployment[] {
  const duplicateCounts = new Map<string, number>();
  for (const deployment of deployments) {
    const name = deployment.name.trim() || "Agent";
    const key = name.toLocaleLowerCase();
    duplicateCounts.set(key, (duplicateCounts.get(key) ?? 0) + 1);
  }

  return deployments.map((deployment) => {
    const name = deployment.name.trim() || "Agent";
    const duplicate = (duplicateCounts.get(name.toLocaleLowerCase()) ?? 0) > 1;
    return {
      name: duplicate ? `${name} · ${compactPubkey(deployment.pubkey)}` : name,
      pubkey: deployment.pubkey,
    };
  });
}
