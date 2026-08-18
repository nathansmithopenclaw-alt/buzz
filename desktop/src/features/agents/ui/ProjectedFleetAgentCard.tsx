import { EllipsisVertical, MessageSquare } from "lucide-react";

import {
  fleetAgentObserverStatus,
  type FleetProjectionAgentPresentation,
} from "@/features/agents/fleetProjection/selectors";
import type { FleetProjectionOwner } from "@/features/agents/fleetProjection/types";
import type { ProfilePanelOpenOptions } from "@/shared/context/ProfilePanelContext";
import { Badge } from "@/shared/ui/badge";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { AgentIdentityCard } from "./AgentIdentityCard";
import { fleetHostLabel, formatFleetRelativeTime } from "./fleetProjectionUi";

function observerStatusLabel(
  status: ReturnType<typeof fleetAgentObserverStatus>,
  now: number,
) {
  if (status.state === "live") return "live";
  if (status.state === "host-unreachable") {
    return status.lastSeen
      ? `host unreachable · last seen ${formatFleetRelativeTime(status.lastSeen, now)}`
      : "host unreachable";
  }
  return status.lastSeen
    ? `quiet · last seen ${formatFleetRelativeTime(status.lastSeen, now)}`
    : "quiet · last seen not reported";
}

export function ProjectedFleetAgentCard({
  agent,
  isMessagePending,
  now,
  owner,
  showHost,
  onMessageAgent,
  onOpenAgentProfile,
}: {
  agent: FleetProjectionAgentPresentation;
  isMessagePending: boolean;
  now: number;
  owner: FleetProjectionOwner;
  showHost: boolean;
  onMessageAgent: (pubkey: string) => void;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
}) {
  const status = fleetAgentObserverStatus(owner, agent.agentPubkey, now);
  const statusLabel = observerStatusLabel(status, now);
  const runtimeLabel = agent.personaRuntime?.trim() || "Runtime not reported";
  const modelLabel = agent.personaModel?.trim() || "Model not reported";

  return (
    <AgentIdentityCard
      actions={
        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <button
              aria-label={`Open actions for ${agent.displayName}`}
              className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
              type="button"
            >
              <EllipsisVertical className="h-4 w-4" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="end"
            onCloseAutoFocus={(event) => event.preventDefault()}
          >
            <DropdownMenuItem
              disabled={isMessagePending}
              onSelect={() => onMessageAgent(agent.agentPubkey)}
            >
              <MessageSquare className="h-4 w-4" />
              Message
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      }
      ariaLabel={`${agent.displayName} agent profile`}
      avatarUrl={agent.avatarUrl}
      className={
        status.state === "host-unreachable" ? "opacity-60 grayscale" : undefined
      }
      dataTestId={`fleet-agent-${agent.agentPubkey}`}
      label={agent.displayName}
      modelLabel={`${runtimeLabel} · ${modelLabel}`}
      onClick={() => onOpenAgentProfile(agent.agentPubkey)}
      statusBadge={
        <div className="flex flex-wrap gap-1.5">
          <Badge
            className="normal-case tracking-normal"
            data-testid={`fleet-agent-status-${agent.agentPubkey}`}
            variant={status.state === "live" ? "success" : "secondary"}
          >
            {statusLabel}
          </Badge>
          {showHost ? (
            <Badge className="normal-case tracking-normal" variant="outline">
              {status.host ? fleetHostLabel(status.host) : "Host not reported"}
            </Badge>
          ) : null}
        </div>
      }
    />
  );
}
