import type { ChannelType } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { FleetProjectionAgent, FleetProjectionOwner } from "./types";

export function projectionAgentCanBeMentioned({
  agent,
  channelType,
  currentPubkey,
  memberPubkeys,
}: {
  agent: FleetProjectionAgent;
  channelType?: ChannelType | null;
  currentPubkey: string | null;
  memberPubkeys: ReadonlySet<string>;
}) {
  const viewerPubkey = currentPubkey ? normalizePubkey(currentPubkey) : null;
  if (viewerPubkey && agent.ownerPubkey === viewerPubkey) return true;
  if (channelType !== "stream" && channelType !== "forum") return false;
  if (!memberPubkeys.has(normalizePubkey(agent.agentPubkey))) return false;
  if (agent.respondTo === "anyone") return true;
  return (
    agent.respondTo === "allowlist" &&
    Boolean(viewerPubkey && agent.respondToAllowlist.includes(viewerPubkey))
  );
}

export function mentionableProjectionAgents({
  owners,
  channelType,
  currentPubkey,
  memberPubkeys,
}: {
  owners: readonly FleetProjectionOwner[];
  channelType?: ChannelType | null;
  currentPubkey: string | null;
  memberPubkeys: ReadonlySet<string>;
}) {
  return owners.flatMap((owner) =>
    owner.agents.filter((agent) =>
      projectionAgentCanBeMentioned({
        agent,
        channelType,
        currentPubkey,
        memberPubkeys,
      }),
    ),
  );
}
