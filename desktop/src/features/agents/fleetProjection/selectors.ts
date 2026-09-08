import { resolveSelectedAvatar } from "@/shared/lib/selectedAvatar";
import type { RelayAgent, UserProfileSummary } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type {
  FleetProjectionAgent,
  FleetProjectionHost,
  FleetProjectionLifecycleState,
  FleetProjectionOwner,
} from "./types";

export const FLEET_HEARTBEAT_FRESHNESS_SECONDS = 90;

export type FleetProjectionAgentPresentation = FleetProjectionAgent & {
  displayName: string;
  avatarUrl: string | null;
};

export type FleetAgentObserverStatus = {
  state: "live" | "quiet" | "host-unreachable";
  lastSeen: number | null;
  host: FleetProjectionHost | null;
};

export type FleetAgentLiveness = {
  state: FleetProjectionLifecycleState | "unknown";
  since: number | null;
  stale: boolean;
};

export function presentFleetProjectionAgent(
  agent: FleetProjectionAgent,
  profile: UserProfileSummary | null | undefined,
  relayAgent: RelayAgent | null | undefined,
): FleetProjectionAgentPresentation {
  const profileName =
    profile?.displayName?.trim() ||
    profile?.name?.trim() ||
    profile?.nip05Handle?.trim() ||
    null;
  return {
    ...agent,
    displayName:
      agent.personaDisplayName?.trim() ||
      profileName ||
      relayAgent?.name.trim() ||
      agent.name,
    avatarUrl: resolveSelectedAvatar(
      agent.selectedAvatarUrl,
      agent.personaAvatarUrl,
      profile?.avatarUrl,
    ),
  };
}

export function primaryFleetHost(
  owner: FleetProjectionOwner | null | undefined,
): FleetProjectionHost | null {
  return owner?.hosts[0] ?? null;
}

export function fleetHostIsFresh(
  host: FleetProjectionHost | null | undefined,
  nowSeconds: number,
) {
  if (!host?.status) return false;
  return (
    Math.max(0, nowSeconds - host.status.timestamp) <=
    FLEET_HEARTBEAT_FRESHNESS_SECONDS
  );
}

export function fleetAgentLiveness(
  owner: FleetProjectionOwner,
  agentPubkey: string,
  nowSeconds: number,
): FleetAgentLiveness {
  const normalized = normalizePubkey(agentPubkey);
  const host = owner.hosts.find((candidate) =>
    candidate.status?.agents.some((agent) => agent.pubkey === normalized),
  );
  if (!host?.status) {
    return { state: "unknown", since: null, stale: false };
  }
  if (!fleetHostIsFresh(host, nowSeconds)) {
    return { state: "unknown", since: null, stale: true };
  }
  const status = host.status.agents.find(
    (agent) => agent.pubkey === normalized,
  );
  return status
    ? { state: status.state, since: status.since, stale: false }
    : { state: "unknown", since: null, stale: false };
}

export function fleetAgentHost(
  owner: FleetProjectionOwner,
  agentPubkey: string,
): FleetProjectionHost | null {
  const normalized = normalizePubkey(agentPubkey);
  return (
    owner.hosts.find((host) =>
      host.status?.agents.some((agent) => agent.pubkey === normalized),
    ) ?? null
  );
}

export function fleetAgentObserverStatus(
  owner: FleetProjectionOwner,
  agentPubkey: string,
  nowSeconds: number,
): FleetAgentObserverStatus {
  const host = fleetAgentHost(owner, agentPubkey);
  if (!host?.status) return { state: "quiet", lastSeen: null, host };
  if (!fleetHostIsFresh(host, nowSeconds)) {
    return {
      state: "host-unreachable",
      lastSeen: host.status.timestamp,
      host,
    };
  }
  const normalized = normalizePubkey(agentPubkey);
  const report = host.status.agents.find(
    (agent) => agent.pubkey === normalized,
  );
  if (report?.state === "running") {
    return { state: "live", lastSeen: host.status.timestamp, host };
  }
  return { state: "quiet", lastSeen: host.status.timestamp, host };
}
