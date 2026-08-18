import { RadioTower } from "lucide-react";

import { fleetHostIsFresh } from "@/features/agents/fleetProjection/selectors";
import type { FleetProjectionHost } from "@/features/agents/fleetProjection/types";
import { fleetHostLabel, formatFleetRelativeTime } from "./fleetProjectionUi";

function hostStatusCopy(host: FleetProjectionHost, now: number) {
  if (!host.status) return "Status not reported";
  if (!fleetHostIsFresh(host, now)) {
    return `Host unreachable · last seen ${formatFleetRelativeTime(host.status.timestamp, now)}`;
  }
  return `Heartbeat ${formatFleetRelativeTime(host.status.timestamp, now)}`;
}

export function FleetHostStatus({
  hosts,
  now,
}: {
  hosts: readonly FleetProjectionHost[];
  now: number;
}) {
  if (hosts.length === 0) {
    return (
      <div
        className="flex items-center gap-2 rounded-xl border border-border/70 bg-muted/35 px-4 py-3 text-sm text-muted-foreground"
        data-fresh="false"
        data-testid="fleet-host-status"
      >
        <RadioTower className="h-4 w-4" />
        No fleet host is publishing right now.
      </div>
    );
  }

  const allFresh = hosts.every((host) => fleetHostIsFresh(host, now));
  return (
    <div
      className="rounded-xl border border-border/70 bg-muted/35 px-4 py-3"
      data-fresh={allFresh ? "true" : "false"}
      data-testid="fleet-host-status"
    >
      <div className="flex flex-col gap-2">
        {hosts.map((host) => {
          const fresh = fleetHostIsFresh(host, now);
          return (
            <div
              className="flex items-center gap-2 text-sm"
              data-testid={`fleet-host-${host.hostId}`}
              key={host.hostId}
            >
              <RadioTower
                className={
                  fresh
                    ? "h-4 w-4 text-emerald-500"
                    : "h-4 w-4 text-muted-foreground"
                }
              />
              <span>
                <strong>{fleetHostLabel(host)}</strong>
                <span className="text-muted-foreground">
                  {" "}
                  · {hostStatusCopy(host, now)}
                </span>
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
