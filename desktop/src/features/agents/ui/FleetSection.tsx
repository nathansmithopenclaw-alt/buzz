import {
  EllipsisVertical,
  MessageSquare,
  Play,
  RotateCw,
  Square,
} from "lucide-react";

import { resolveAgentCardModelLabel } from "@/features/agents/lib/agentCardModelLabel";
import { friendlyAgentLastError } from "@/features/agents/lib/friendlyAgentLastError";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { useUserProfileQuery } from "@/features/profile/hooks";
import type { AgentPersona, ManagedAgent } from "@/shared/api/types";
import type { ProfilePanelOpenOptions } from "@/shared/context/ProfilePanelContext";
import { Badge } from "@/shared/ui/badge";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { IdentityCardSkeleton } from "@/shared/ui/identity-card-skeleton";
import { SectionHeader } from "@/shared/ui/PageHeader";
import { AgentIdentityCard } from "./AgentIdentityCard";
import { AgentRuntimeAvatarControl } from "./AgentRuntimeAvatarControl";
import { IDENTITY_CARD_GRID_CLASS } from "./agentCardLayout";
import {
  resolveFleetAgentIdentity,
  resolveFleetAgentRuntimeLabel,
} from "./agentFleetIdentity";

type FleetSectionProps = {
  agents: ManagedAgent[];
  defaultModel: string;
  error: Error | null;
  isActionPending: boolean;
  isLoading: boolean;
  isMessagePending: boolean;
  personasById: ReadonlyMap<string, AgentPersona>;
  restartingAgentPubkey: string | null;
  startingAgentPubkey: string | null;
  onMessageAgent: (pubkey: string) => void;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  onRestartAgent: (pubkey: string) => void;
  onStartAgent: (pubkey: string) => void;
  onStopAgent: (pubkey: string) => void;
};

export function FleetSection({
  agents,
  defaultModel,
  error,
  isActionPending,
  isLoading,
  isMessagePending,
  personasById,
  restartingAgentPubkey,
  startingAgentPubkey,
  onMessageAgent,
  onOpenAgentProfile,
  onRestartAgent,
  onStartAgent,
  onStopAgent,
}: FleetSectionProps) {
  return (
    <section className="relative space-y-4" data-testid="agents-fleet">
      <SectionHeader
        description="Running and stopped agent instances on this device."
        title="Fleet"
      />

      {isLoading ? <FleetLoadingSkeleton /> : null}

      {!isLoading && !error && agents.length === 0 ? (
        <div className="rounded-2xl border border-dashed border-border/80 px-5 py-6">
          <p className="font-medium">No agent instances yet</p>
          <p className="mt-1 text-sm text-muted-foreground">
            Deploy a template from the Library to add it to your fleet.
          </p>
        </div>
      ) : null}

      {!isLoading && agents.length > 0 ? (
        <div className={IDENTITY_CARD_GRID_CLASS}>
          {agents.map((agent) => (
            <FleetAgentCard
              agent={agent}
              defaultModel={defaultModel}
              isActionPending={isActionPending}
              isMessagePending={isMessagePending}
              key={agent.pubkey}
              persona={
                agent.personaId ? personasById.get(agent.personaId) : undefined
              }
              restartingAgentPubkey={restartingAgentPubkey}
              startingAgentPubkey={startingAgentPubkey}
              onMessageAgent={onMessageAgent}
              onOpenAgentProfile={onOpenAgentProfile}
              onRestartAgent={onRestartAgent}
              onStartAgent={onStartAgent}
              onStopAgent={onStopAgent}
            />
          ))}
        </div>
      ) : null}

      {error ? (
        <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
          {error.message}
        </p>
      ) : null}
    </section>
  );
}

function FleetAgentCard({
  agent,
  defaultModel,
  isActionPending,
  isMessagePending,
  persona,
  restartingAgentPubkey,
  startingAgentPubkey,
  onMessageAgent,
  onOpenAgentProfile,
  onRestartAgent,
  onStartAgent,
  onStopAgent,
}: {
  agent: ManagedAgent;
  defaultModel: string;
  isActionPending: boolean;
  isMessagePending: boolean;
  persona?: AgentPersona;
  restartingAgentPubkey: string | null;
  startingAgentPubkey: string | null;
  onMessageAgent: (pubkey: string) => void;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  onRestartAgent: (pubkey: string) => void;
  onStartAgent: (pubkey: string) => void;
  onStopAgent: (pubkey: string) => void;
}) {
  const profileQuery = useUserProfileQuery(agent.pubkey);
  const identity = resolveFleetAgentIdentity({
    agent,
    persona,
    profile: profileQuery.data,
  });
  const friendlyError = friendlyAgentLastError(
    agent.lastError,
    agent.lastErrorCode,
  )?.copy;
  const isActive = isManagedAgentActive(agent);
  const isRestarting = restartingAgentPubkey === agent.pubkey;
  const isStarting = startingAgentPubkey === agent.pubkey;
  const opensRuntimeTab = Boolean(friendlyError && !isActive);
  const status = isActive ? "running" : friendlyError ? "error" : "stopped";
  const runtimeLabel = resolveFleetAgentRuntimeLabel({ agent, persona });
  const modelLabel = resolveAgentCardModelLabel({
    agent,
    personaModel: persona?.model,
    defaultModel,
  });

  return (
    <AgentIdentityCard
      actions={
        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <button
              aria-label={`Open actions for ${identity.displayName}`}
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
              onSelect={() => onMessageAgent(agent.pubkey)}
            >
              <MessageSquare className="h-4 w-4" />
              Message
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            {isActive ? (
              <>
                <DropdownMenuItem
                  disabled={isActionPending}
                  onSelect={() => onStopAgent(agent.pubkey)}
                >
                  <Square className="h-4 w-4" />
                  Stop
                </DropdownMenuItem>
                <DropdownMenuItem
                  disabled={isActionPending}
                  onSelect={() => onRestartAgent(agent.pubkey)}
                >
                  <RotateCw className="h-4 w-4" />
                  Restart
                </DropdownMenuItem>
              </>
            ) : (
              <DropdownMenuItem
                disabled={isActionPending}
                onSelect={() =>
                  agent.needsRestart
                    ? onRestartAgent(agent.pubkey)
                    : onStartAgent(agent.pubkey)
                }
              >
                {agent.needsRestart ? (
                  <RotateCw className="h-4 w-4" />
                ) : (
                  <Play className="h-4 w-4" />
                )}
                {agent.needsRestart ? "Restart" : "Start"}
              </DropdownMenuItem>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      }
      ariaLabel={`${identity.displayName} agent profile`}
      avatar={
        <AgentRuntimeAvatarControl
          activeTestId={`agent-runtime-active-${agent.pubkey}`}
          avatarUrl={identity.avatarUrl}
          errorLabel={friendlyError}
          errorTestId={`agent-runtime-error-${agent.pubkey}`}
          isActive={isActive}
          isRestarting={isRestarting}
          isStarting={isStarting}
          label={identity.displayName}
          requiresRestart={agent.needsRestart}
          startTestId={`agent-runtime-start-${agent.pubkey}`}
          onOpenError={() => {
            onOpenAgentProfile(agent.pubkey, { tab: "runtime" });
          }}
          onStart={() =>
            agent.needsRestart
              ? onRestartAgent(agent.pubkey)
              : onStartAgent(agent.pubkey)
          }
        />
      }
      avatarUrl={identity.avatarUrl}
      dataTestId={`managed-agent-${agent.pubkey}`}
      label={identity.displayName}
      modelLabel={`${runtimeLabel} · ${modelLabel}`}
      onClick={() => {
        onOpenAgentProfile(
          agent.pubkey,
          opensRuntimeTab ? { tab: "runtime" } : undefined,
        );
      }}
      statusBadge={
        <div className="flex flex-wrap gap-1.5">
          <Badge
            className="normal-case tracking-normal"
            variant={
              status === "running"
                ? "success"
                : status === "error"
                  ? "destructive"
                  : "secondary"
            }
          >
            {status}
          </Badge>
          {agent.personaOrphaned || (agent.personaId && !persona) ? (
            <Badge className="normal-case tracking-normal" variant="warning">
              No definition on this device
            </Badge>
          ) : null}
        </div>
      }
    />
  );
}

function FleetLoadingSkeleton() {
  return (
    <div className={IDENTITY_CARD_GRID_CLASS}>
      <IdentityCardSkeleton
        footerSubtitleWidthClass="w-14"
        footerTitleWidthClass="w-24"
      />
      <IdentityCardSkeleton
        footerSubtitleWidthClass="w-20"
        footerTitleWidthClass="w-32"
      />
      <IdentityCardSkeleton
        footerSubtitleWidthClass="w-16"
        footerTitleWidthClass="w-28"
      />
    </div>
  );
}
