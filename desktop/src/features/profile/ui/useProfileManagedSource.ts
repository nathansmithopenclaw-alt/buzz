import * as React from "react";

import { useCanonicalManagedAgentProfile } from "@/features/profile/lib/useCanonicalManagedAgentProfile";
import type { AgentPersona, ManagedAgent } from "@/shared/api/types";

export function useProfileManagedSource({
  isRelayOnly,
  managedAgents,
  persona,
  personas,
  pubkey,
}: {
  isRelayOnly: boolean;
  managedAgents?: ManagedAgent[];
  persona?: AgentPersona;
  personas?: AgentPersona[];
  pubkey?: string;
}) {
  const canonical = useCanonicalManagedAgentProfile({
    managedAgents: isRelayOnly ? undefined : managedAgents,
    personaId: isRelayOnly ? undefined : persona?.id,
    pubkey,
  });
  const instanceBuckets = isRelayOnly
    ? { archived: [], live: [] }
    : canonical.instanceBuckets;
  const managedAgent = isRelayOnly ? undefined : canonical.managedAgent;
  const resolvedPersonaFromSource = React.useMemo(() => {
    if (pubkey && !managedAgent) return undefined;
    const personaId = canonical.linkedPersonaId ?? managedAgent?.personaId;
    const refreshed = personaId
      ? personas?.find((candidate) => candidate.id === personaId)
      : undefined;
    if (refreshed) return refreshed;
    if (!pubkey && !isRelayOnly && persona) return persona;
    return managedAgent?.personaId
      ? personas?.find((candidate) => candidate.id === managedAgent.personaId)
      : undefined;
  }, [
    canonical.linkedPersonaId,
    isRelayOnly,
    managedAgent,
    persona,
    personas,
    pubkey,
  ]);
  return { instanceBuckets, managedAgent, resolvedPersonaFromSource };
}
