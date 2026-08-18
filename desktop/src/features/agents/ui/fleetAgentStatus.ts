import { friendlyAgentLastError } from "@/features/agents/lib/friendlyAgentLastError";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import type { ManagedAgent } from "@/shared/api/types";
import { formatFleetRelativeTime } from "./fleetProjectionUi";

export type FleetAgentStatus =
  | { source: "host"; state: "running" | "stopped" | "error" }
  | {
      source: "observer";
      state: "live" | "quiet" | "host-unreachable";
      lastSeen: number | null;
      now: number;
    };

export type FleetAgentStatusPresentation = {
  label: string;
  stale: boolean;
  variant: "destructive" | "secondary" | "success";
};

export function hostFleetAgentStatus(
  agent: Pick<ManagedAgent, "lastError" | "lastErrorCode" | "status">,
): FleetAgentStatus {
  if (isManagedAgentActive(agent)) {
    return { source: "host", state: "running" };
  }
  if (friendlyAgentLastError(agent.lastError, agent.lastErrorCode)) {
    return { source: "host", state: "error" };
  }
  return { source: "host", state: "stopped" };
}

export function fleetAgentStatusPresentation(
  status: FleetAgentStatus,
): FleetAgentStatusPresentation {
  if (status.source === "host") {
    return {
      label: status.state,
      stale: false,
      variant:
        status.state === "running"
          ? "success"
          : status.state === "error"
            ? "destructive"
            : "secondary",
    };
  }

  if (status.state === "live") {
    return { label: "live", stale: false, variant: "success" };
  }

  const lastSeen = status.lastSeen
    ? `last seen ${formatFleetRelativeTime(status.lastSeen, status.now)}`
    : "last seen not reported";
  if (status.state === "host-unreachable") {
    return {
      label: status.lastSeen
        ? `host unreachable · ${lastSeen}`
        : "host unreachable",
      stale: true,
      variant: "secondary",
    };
  }

  return {
    label: `quiet · ${lastSeen}`,
    stale: false,
    variant: "secondary",
  };
}
