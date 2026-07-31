import type { AgentManagementRequest } from "./agentManagement";
import type { Channel, ManagedAgent } from "@/shared/api/types";

export type QueuedAgentManagementRequest = {
  agentPubkey: string;
  request: AgentManagementRequest;
};

/**
 * Records an accepted request exactly once and leaves it queued until the
 * current review or automatic create finishes.
 */
export function enqueueAgentManagementRequest(
  queue: QueuedAgentManagementRequest[],
  seenRequestIds: Set<string>,
  candidate: QueuedAgentManagementRequest,
  maxQueued = 100,
): boolean {
  if (seenRequestIds.has(candidate.request.requestId)) return false;
  seenRequestIds.add(candidate.request.requestId);
  queue.push(candidate);
  if (queue.length > maxQueued) queue.shift();
  return true;
}

export function dequeueAgentManagementRequest(
  queue: QueuedAgentManagementRequest[],
): QueuedAgentManagementRequest | undefined {
  return queue.shift();
}

/**
 * Defers the trust decision until both ownership and channel membership have
 * initialized. A draft may open only when its owned sender and the owner share
 * the claimed originating channel.
 */
export function classifyAgentManagementOrigin(
  agents: readonly Pick<ManagedAgent, "pubkey">[] | undefined,
  channels:
    | readonly Pick<Channel, "id" | "isMember" | "memberPubkeys">[]
    | undefined,
  agentPubkey: string,
  channelId: string,
): "buffer" | "accept" | "reject" {
  if (agents === undefined || channels === undefined) return "buffer";
  const normalizedAgentPubkey = agentPubkey.toLowerCase();
  const isOwnedAgent = agents.some(
    (agent) => agent.pubkey.toLowerCase() === normalizedAgentPubkey,
  );
  const originChannel = channels.find((channel) => channel.id === channelId);
  return isOwnedAgent &&
    originChannel?.isMember === true &&
    originChannel.memberPubkeys.some(
      (pubkey) => pubkey.toLowerCase() === normalizedAgentPubkey,
    )
    ? "accept"
    : "reject";
}
