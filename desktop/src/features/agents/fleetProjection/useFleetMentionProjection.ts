import * as React from "react";

import { shouldBlockPersonaCreation } from "@/features/agents/lib/fleetVisibility";
import { useFleetVisibilitySettings } from "@/features/agents/lib/useFleetVisibilitySettings";
import type {
  AgentPersona,
  ChannelMember,
  ChannelType,
  ManagedAgent,
} from "@/shared/api/types";
import { useFeatureEnabled } from "@/shared/features";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { mentionableProjectionAgents } from "./mentions";
import { useFleetProjectionSnapshot } from "./sync";

export function useFleetMentionProjection({
  channelType,
  currentPubkey,
  managedAgents,
  members,
  personas,
}: {
  channelType?: ChannelType | null;
  currentPubkey: string | null;
  managedAgents: readonly ManagedAgent[];
  members: readonly ChannelMember[];
  personas: readonly AgentPersona[];
}) {
  const experimentEnabled = useFeatureEnabled("fleetVisibility");
  const settings = useFleetVisibilitySettings();
  const fleetProjection = useFleetProjectionSnapshot();
  const enabled = experimentEnabled && settings?.enabled === true;
  const observerMode = enabled && settings.role === "observer";
  const normalizedCurrentPubkey = currentPubkey
    ? normalizePubkey(currentPubkey)
    : null;
  const memberPubkeys = React.useMemo(
    () => new Set(members.map((member) => normalizePubkey(member.pubkey))),
    [members],
  );
  const projectionOwners = React.useMemo(
    () => (enabled ? fleetProjection.owners : []),
    [enabled, fleetProjection.owners],
  );
  const projectionAgents = React.useMemo(
    () => projectionOwners.flatMap((owner) => owner.agents),
    [projectionOwners],
  );
  const eligibleProjectionAgents = React.useMemo(
    () =>
      mentionableProjectionAgents({
        owners: projectionOwners,
        channelType,
        currentPubkey,
        memberPubkeys,
      }),
    [channelType, currentPubkey, memberPubkeys, projectionOwners],
  );
  const localManagedAgents = React.useMemo(
    () => (observerMode ? [] : managedAgents),
    [managedAgents, observerMode],
  );
  const managedAgentNamesByPubkey = React.useMemo(
    () =>
      new Map([
        ...localManagedAgents.map(
          (agent) => [normalizePubkey(agent.pubkey), agent.name] as const,
        ),
        ...projectionAgents
          .filter((agent) => agent.ownerPubkey === normalizedCurrentPubkey)
          .map(
            (agent) =>
              [normalizePubkey(agent.agentPubkey), agent.name] as const,
          ),
      ]),
    [localManagedAgents, normalizedCurrentPubkey, projectionAgents],
  );
  const managedAgentPersonaIdsByPubkey = React.useMemo(
    () =>
      new Map([
        ...localManagedAgents
          .filter((agent) => Boolean(agent.personaId))
          .map(
            (agent) =>
              [
                normalizePubkey(agent.pubkey),
                agent.personaId as string,
              ] as const,
          ),
        ...projectionAgents
          .filter((agent) => Boolean(agent.personaId))
          .map(
            (agent) =>
              [
                normalizePubkey(agent.agentPubkey),
                agent.personaId as string,
              ] as const,
          ),
      ]),
    [localManagedAgents, projectionAgents],
  );
  const managedAgentPersonaIds = React.useMemo(
    () =>
      new Set(
        [
          ...localManagedAgents.map((agent) => agent.personaId),
          ...projectionAgents
            .filter((agent) => agent.ownerPubkey === normalizedCurrentPubkey)
            .map((agent) => agent.personaId),
        ].filter((personaId): personaId is string => Boolean(personaId)),
      ),
    [localManagedAgents, normalizedCurrentPubkey, projectionAgents],
  );
  const managedAgentPubkeys = React.useMemo(
    () =>
      new Set(
        [
          ...localManagedAgents.map((agent) => agent.pubkey),
          ...projectionAgents
            .filter((agent) => agent.ownerPubkey === normalizedCurrentPubkey)
            .map((agent) => agent.agentPubkey),
        ].map(normalizePubkey),
      ),
    [localManagedAgents, normalizedCurrentPubkey, projectionAgents],
  );
  const personaNameByPubkey = React.useMemo(() => {
    const personaById = new Map(
      personas.map((persona) => [persona.id, persona.displayName]),
    );
    const lookup = new Map<string, string>();
    for (const agent of localManagedAgents) {
      const name = agent.personaId ? personaById.get(agent.personaId) : null;
      if (name) lookup.set(normalizePubkey(agent.pubkey), name);
    }
    for (const agent of projectionAgents) {
      const name = agent.personaDisplayName?.trim();
      if (name) lookup.set(normalizePubkey(agent.agentPubkey), name);
    }
    return lookup;
  }, [localManagedAgents, personas, projectionAgents]);

  return {
    blockPersonaCreation: shouldBlockPersonaCreation(
      experimentEnabled,
      settings,
    ),
    eligibleProjectionAgents,
    localManagedAgents,
    managedAgentNamesByPubkey,
    managedAgentPersonaIds,
    managedAgentPersonaIdsByPubkey,
    managedAgentPubkeys,
    memberPubkeys,
    personaNameByPubkey,
    projectionAgents,
  };
}
