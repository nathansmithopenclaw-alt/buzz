import * as React from "react";

import { useCanonicalManagedAgentProfile } from "@/features/profile/lib/useCanonicalManagedAgentProfile";
import type { AgentPersona, ManagedAgent } from "@/shared/api/types";

export function useProfileManagedSource({
  currentPubkey,
  isRelayOnly,
  managedAgents,
  persona,
  personas,
  preserveRequestedInstance,
  pubkey,
}: {
  currentPubkey?: string;
  isRelayOnly: boolean;
  managedAgents?: ManagedAgent[];
  persona?: AgentPersona;
  personas?: AgentPersona[];
  preserveRequestedInstance: boolean;
  pubkey?: string;
}) {
  const canonical = useCanonicalManagedAgentProfile({
    currentPubkey,
    managedAgents: isRelayOnly ? undefined : managedAgents,
    personaId: isRelayOnly ? undefined : persona?.id,
    preferDirectManagedAgent: true,
    preserveRequestedInstance,
    pubkey,
  });
  const instanceBuckets = isRelayOnly
    ? { archived: [], live: [] }
    : canonical.instanceBuckets;
  const managedAgent = isRelayOnly ? undefined : canonical.managedAgent;
  const resolvedPersonaFromSource = React.useMemo(() => {
    const personaId = canonical.linkedPersonaId ?? managedAgent?.personaId;
    const refreshed = personaId
      ? personas?.find((candidate) => candidate.id === personaId)
      : undefined;
    if (refreshed) return refreshed;
    if (!isRelayOnly && persona) return persona;
    return managedAgent?.personaId
      ? personas?.find((candidate) => candidate.id === managedAgent.personaId)
      : undefined;
  }, [canonical.linkedPersonaId, isRelayOnly, managedAgent, persona, personas]);
  return { instanceBuckets, managedAgent, resolvedPersonaFromSource };
}
