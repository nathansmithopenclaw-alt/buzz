import * as React from "react";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  inboundResurrectionTargets,
  subscribeInboundLiveEvents,
} from "@/features/agents/lib/inboundAgentResurrection";
import { channelsQueryKey } from "@/features/channels/hooks";
import { ensureManagedAgentForInbound } from "@/shared/api/tauriManagedAgents";
import type { Channel, RelayEvent } from "@/shared/api/types";
import { useQuery } from "@tanstack/react-query";

const SEEN_LIMIT = 2_000;

function trackSeen(seen: Set<string>, id: string): boolean {
  if (seen.has(id)) return false;
  seen.add(id);
  if (seen.size > SEEN_LIMIT) {
    const first = seen.values().next().value;
    if (first) seen.delete(first);
  }
  return true;
}

/**
 * When a live channel/DM event mentions a stopped local managed agent,
 * ask Rust to spawn it (respond_to + archived + crash-loop gated).
 */
export function useInboundAgentResurrection() {
  const agents = useManagedAgentsQuery().data;
  const channels = useQuery({ queryKey: channelsQueryKey }).data as
    | Channel[]
    | undefined;
  const agentsRef = React.useRef(agents);
  const channelsRef = React.useRef(channels);
  agentsRef.current = agents;
  channelsRef.current = channels;
  const seenRef = React.useRef(new Set<string>());
  const inFlightRef = React.useRef(new Set<string>());

  const considerEvent = React.useEffectEvent((event: RelayEvent) => {
    if (!trackSeen(seenRef.current, event.id)) return;
    const currentAgents = agentsRef.current;
    if (!currentAgents?.length) return;
    const targets = inboundResurrectionTargets(event, currentAgents);
    if (targets.length === 0) return;
    const channelId = event.tags.find((tag) => tag[0] === "h")?.[1];
    const known = channelId
      ? channelsRef.current?.find((channel) => channel.id === channelId)
      : undefined;
    // Unknown channel type is treated as a DM so allowlist/anyone cannot
    // transitively grant access (same fail-closed rule as buzz-acp).
    const isDm = known?.channelType !== "stream" && known?.channelType !== "forum";
    for (const pubkey of targets) {
      const key = `${pubkey}:${event.id}`;
      if (inFlightRef.current.has(key)) continue;
      inFlightRef.current.add(key);
      void ensureManagedAgentForInbound({
        pubkey,
        authorPubkey: event.pubkey,
        isDm,
      }).finally(() => {
        inFlightRef.current.delete(key);
      });
    }
  });

  React.useEffect(() => {
    return subscribeInboundLiveEvents((event: RelayEvent) => {
      considerEvent(event);
    });
  }, []);
}
