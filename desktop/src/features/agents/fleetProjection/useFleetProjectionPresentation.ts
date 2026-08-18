import * as React from "react";

import { useRelayAgentsQuery } from "@/features/agents/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import {
  presentFleetProjectionAgent,
  type FleetProjectionAgentPresentation,
} from "./selectors";
import { useFleetProjectionSnapshot } from "./sync";
import type { FleetProjectionOwner } from "./types";

export type FleetProjectionOwnerPresentation = Omit<
  FleetProjectionOwner,
  "agents"
> & {
  agents: readonly FleetProjectionAgentPresentation[];
  displayName: string;
};

export function useFleetProjectionPresentation(enabled = true) {
  const snapshot = useFleetProjectionSnapshot();
  const pubkeys = React.useMemo(
    () =>
      enabled
        ? [
            ...new Set(
              snapshot.owners.flatMap((owner) => [
                owner.ownerPubkey,
                ...owner.agents.map((agent) => agent.agentPubkey),
              ]),
            ),
          ]
        : [],
    [enabled, snapshot.owners],
  );
  const profilesQuery = useUsersBatchQuery(pubkeys, {
    enabled: pubkeys.length > 0,
  });
  const relayAgentsQuery = useRelayAgentsQuery({ enabled: pubkeys.length > 0 });
  const relayAgentsByPubkey = React.useMemo(
    () =>
      new Map(
        (relayAgentsQuery.data ?? []).map((agent) => [
          normalizePubkey(agent.pubkey),
          agent,
        ]),
      ),
    [relayAgentsQuery.data],
  );
  const owners = React.useMemo<FleetProjectionOwnerPresentation[]>(
    () =>
      enabled
        ? snapshot.owners.map((owner) => {
            const profile = profilesQuery.data?.profiles[owner.ownerPubkey];
            return {
              ...owner,
              agents: owner.agents.map((agent) =>
                presentFleetProjectionAgent(
                  agent,
                  profilesQuery.data?.profiles[
                    normalizePubkey(agent.agentPubkey)
                  ],
                  relayAgentsByPubkey.get(normalizePubkey(agent.agentPubkey)),
                ),
              ),
              displayName:
                profile?.displayName?.trim() ||
                profile?.name?.trim() ||
                profile?.nip05Handle?.trim() ||
                truncatePubkey(owner.ownerPubkey),
            };
          })
        : [],
    [
      enabled,
      profilesQuery.data?.profiles,
      relayAgentsByPubkey,
      snapshot.owners,
    ],
  );

  return {
    ...snapshot,
    loading: enabled && snapshot.loading,
    errorMessage: enabled ? snapshot.errorMessage : null,
    owners,
  };
}
