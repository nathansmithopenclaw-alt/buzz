import type { AgentPersona, ManagedAgent, Profile } from "@/shared/api/types";

type FleetIdentityInput = {
  agent: Pick<ManagedAgent, "avatarUrl" | "name" | "pubkey">;
  persona?: Pick<AgentPersona, "avatarUrl" | "displayName">;
  profile?: Pick<Profile, "avatarUrl" | "displayName" | "hasProfileEvent">;
};

type FleetRuntimeInput = {
  agent: Pick<ManagedAgent, "runtime">;
  persona?: Pick<AgentPersona, "runtime">;
};

function trimmed(value: string | null | undefined) {
  return value?.trim() || null;
}

function compactPubkey(pubkey: string) {
  const value = pubkey.trim();
  if (value.length <= 12) return value || "Agent";
  return `${value.slice(0, 6)}…${value.slice(-6)}`;
}

export function resolveFleetAgentIdentity({
  agent,
  persona,
  profile,
}: FleetIdentityInput) {
  const profileDisplayName = profile?.hasProfileEvent
    ? trimmed(profile.displayName)
    : null;
  const profileAvatarUrl = profile?.hasProfileEvent
    ? trimmed(profile.avatarUrl)
    : null;

  return {
    avatarUrl:
      profileAvatarUrl ??
      trimmed(persona?.avatarUrl) ??
      trimmed(agent.avatarUrl),
    displayName:
      profileDisplayName ??
      trimmed(persona?.displayName) ??
      trimmed(agent.name) ??
      compactPubkey(agent.pubkey),
  };
}

export function resolveFleetAgentRuntimeLabel({
  agent,
  persona,
}: FleetRuntimeInput) {
  return (
    trimmed(agent.runtime) ?? trimmed(persona?.runtime) ?? "Default runtime"
  );
}
