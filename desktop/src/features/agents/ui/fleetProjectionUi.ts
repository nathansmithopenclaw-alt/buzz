import type { FleetProjectionHost } from "@/features/agents/fleetProjection/types";

export function formatFleetRelativeTime(timestamp: number, now: number) {
  const seconds = Math.max(0, now - timestamp);
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

export function fleetHostLabel(host: FleetProjectionHost | null | undefined) {
  return host?.name?.trim() || "Unnamed host";
}
