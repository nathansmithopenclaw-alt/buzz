import { AgentManagementMarker } from "./OtherSetupAgentMarker";
import { Activity, EllipsisVertical, MessageSquare } from "lucide-react";

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
import { FleetAgentStatusChip } from "./FleetAgentStatusChip";
import { fleetHostLabel } from "./fleetProjectionUi";

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
          <DropdownMenuContent align="end">
            <DropdownMenuItem
              disabled={isMessagePending}
              onSelect={() => onMessageAgent(agent.agentPubkey)}
            >
              <MessageSquare className="h-4 w-4" />
              Message
            </DropdownMenuItem>
            <DropdownMenuItem
              onSelect={() =>
                onOpenAgentProfile(agent.agentPubkey, {
                  source: "relay",
                  view: "activity",
                })
              }
            >
              <Activity className="h-4 w-4" />
              View activity
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
      footerAccessory={<AgentManagementMarker pubkey={agent.agentPubkey} />}
      label={agent.displayName}
      modelLabel={`${runtimeLabel} · ${modelLabel}`}
      subtitle={agent.personaDescription}
      onClick={() => onOpenAgentProfile(agent.agentPubkey, { source: "relay" })}
      statusBadge={
        <div className="flex flex-wrap gap-1.5">
          <FleetAgentStatusChip
            status={{
              source: "observer",
              state: status.state,
              lastSeen: status.lastSeen,
              now,
            }}
            testId={`fleet-agent-status-${agent.agentPubkey}`}
          />
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
