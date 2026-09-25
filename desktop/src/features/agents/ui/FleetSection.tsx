import {
  Activity,
  EllipsisVertical,
  MessageSquare,
  Play,
  RotateCw,
  Settings2,
  Square,
  TerminalSquare,
} from "lucide-react";

import {
  agentEffortModelQualifier,
  resolveAgentCardModelLabel,
} from "@/features/agents/lib/agentCardModelLabel";
import { useAcpRuntimesQuery } from "@/features/agents/hooks";
import { effectiveAgentDescription } from "@/features/agents/lib/agentDescription";
import { fleetAgentHost } from "@/features/agents/fleetProjection/selectors";
import type { FleetProjectionOwnerPresentation } from "@/features/agents/fleetProjection/useFleetProjectionPresentation";
import { friendlyAgentLastError } from "@/features/agents/lib/friendlyAgentLastError";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import {
  agentPresenceStartBlockReason,
  type AgentAvailabilityReader,
} from "@/features/agents/lib/useAgentAvailability";
import {
  ProtectedBestieCardBadge,
  useProtectedBestiePubkey,
} from "@protected-feature-components";
import { AgentManagementMarker } from "./OtherSetupAgentMarker";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
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

import { AgentIdentityCard } from "./AgentIdentityCard";
import { AgentRuntimeAvatarControl } from "./AgentRuntimeAvatarControl";
import { IDENTITY_CARD_GRID_CLASS } from "./agentCardLayout";
import { fleetHostLabel } from "./fleetProjectionUi";
import { ProjectedFleetAgentCard } from "./ProjectedFleetAgentCard";
import { FleetAgentStatusChip } from "./FleetAgentStatusChip";
import { hostFleetAgentStatus } from "./fleetAgentStatus";
import {
  resolveFleetAgentIdentity,
  resolveFleetAgentRuntimeLabel,
} from "./agentFleetIdentity";
import { PersonaActionItems } from "./PersonaActionsMenu";
import {
  HostUndeployedPersonaCard,
  ObserverUndeployedPersonaCard,
} from "./UndeployedPersonaCard";
import type { FleetProjectionPersona } from "@/features/agents/fleetProjection/types";

type HostFleetSectionProps = {
  mode: "host";
  getAvailability: AgentAvailabilityReader;
  agents: ManagedAgent[];
  defaultModel: string;
  error: Error | null;
  isActionPending: boolean;
  isLoading: boolean;
  isMessagePending: boolean;
  isPersonaPending: boolean;
  personasById: ReadonlyMap<string, AgentPersona>;
  restartingAgentPubkey: string | null;
  startingAgentPubkey: string | null;
  startingPersonaIds: ReadonlySet<string>;
  undeployedPersonas: readonly AgentPersona[];
  onDeactivatePersona: (persona: AgentPersona) => void;
  onDeletePersona: (persona: AgentPersona) => void;
  onDeployPersona: (persona: AgentPersona) => void;
  onDuplicatePersona: (persona: AgentPersona) => void;
  /** `agent` is set when a deployed agent's card opens the editor. */
  onEditPersona: (persona: AgentPersona, agent?: ManagedAgent) => void;
  onMessageAgent: (pubkey: string) => void;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  onOpenPersonaProfile: (persona: AgentPersona) => void;
  onRestartAgent: (pubkey: string) => void;
  onSharePersona: (persona: AgentPersona) => void;
  onSnapshotPersona: (persona: AgentPersona) => void;
  onStartAgent: (pubkey: string) => void;
  onStopAgent: (pubkey: string) => void;
};

type ObserverFleetSectionProps = {
  mode: "observer";
  error: Error | null;
  isLoading: boolean;
  isMessagePending: boolean;
  now: number;
  owner: FleetProjectionOwnerPresentation | null;
  undeployedPersonas: readonly FleetProjectionPersona[];
  emptyTitle?: string;
  emptyDescription?: string;
  onMessageAgent: (pubkey: string) => void;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
};

type FleetSectionProps = HostFleetSectionProps | ObserverFleetSectionProps;

export function FleetSection(props: FleetSectionProps) {
  if (props.mode === "observer") return <ObserverFleetSection {...props} />;
  return <HostFleetSection {...props} />;
}

function HostFleetSection(props: HostFleetSectionProps) {
  const {
    agents,
    getAvailability,
    defaultModel,
    error,
    isActionPending,
    isLoading,
    isMessagePending,
    isPersonaPending,
    personasById,
    restartingAgentPubkey,
    startingAgentPubkey,
    startingPersonaIds,
    undeployedPersonas,
    onDeactivatePersona,
    onDeletePersona,
    onDeployPersona,
    onDuplicatePersona,
    onEditPersona,
    onMessageAgent,
    onOpenAgentProfile,
    onOpenPersonaProfile,
    onRestartAgent,
    onSharePersona,
    onSnapshotPersona,
    onStartAgent,
    onStopAgent,
  } = props;
  const isArchived = useIsArchivedPredicate();
  const bestiePubkey = useProtectedBestiePubkey(agents)?.toLowerCase() ?? null;
  const visibleAgents = agents.filter((agent) => {
    const persona = agent.personaId
      ? personasById.get(agent.personaId)
      : undefined;
    return persona !== undefined || !isArchived(agent.pubkey);
  });
  const hasCards = visibleAgents.length > 0 || undeployedPersonas.length > 0;
  return (
    <section className="relative space-y-4" data-testid="agents-fleet">
      {isLoading ? <FleetLoadingSkeleton /> : null}

      {!isLoading && !error && !hasCards ? (
        <div className="rounded-2xl border border-dashed border-border/80 px-5 py-6">
          <p className="font-medium">No agents yet</p>
          <p className="mt-1 text-sm text-muted-foreground">
            Create an agent to run it on this Mac.
          </p>
        </div>
      ) : null}

      {!isLoading && hasCards ? (
        <div
          className={IDENTITY_CARD_GRID_CLASS}
          data-testid="agents-fleet-grid"
        >
          {visibleAgents.map((agent) => (
            <FleetAgentCard
              agent={agent}
              getAvailability={getAvailability}
              isBestie={agent.pubkey.toLowerCase() === bestiePubkey}
              defaultModel={defaultModel}
              isActionPending={isActionPending}
              isMessagePending={isMessagePending}
              key={agent.pubkey}
              persona={
                agent.personaId ? personasById.get(agent.personaId) : undefined
              }
              restartingAgentPubkey={restartingAgentPubkey}
              startingAgentPubkey={startingAgentPubkey}
              onDeactivatePersona={onDeactivatePersona}
              onDeletePersona={onDeletePersona}
              onDuplicatePersona={onDuplicatePersona}
              onEditPersona={onEditPersona}
              onMessageAgent={onMessageAgent}
              onOpenAgentProfile={onOpenAgentProfile}
              onRestartAgent={onRestartAgent}
              onSharePersona={onSharePersona}
              onSnapshotPersona={onSnapshotPersona}
              onStartAgent={onStartAgent}
              onStopAgent={onStopAgent}
              isPersonaPending={isPersonaPending}
            />
          ))}
          {undeployedPersonas.map((persona) => (
            <HostUndeployedPersonaCard
              defaultModel={defaultModel}
              isActionPending={isActionPending}
              isPending={isPersonaPending}
              isStarting={startingPersonaIds.has(persona.id)}
              key={persona.id}
              persona={persona}
              onDeactivatePersona={onDeactivatePersona}
              onDeletePersona={onDeletePersona}
              onDeployPersona={onDeployPersona}
              onDuplicatePersona={onDuplicatePersona}
              onEditPersona={onEditPersona}
              onOpenPersonaProfile={onOpenPersonaProfile}
              onSharePersona={onSharePersona}
              onSnapshotPersona={onSnapshotPersona}
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

function ObserverFleetSection({
  emptyDescription,
  emptyTitle,
  error,
  isLoading,
  isMessagePending,
  now,
  owner,
  undeployedPersonas,
  onMessageAgent,
  onOpenAgentProfile,
}: ObserverFleetSectionProps) {
  const agents = owner?.agents ?? [];
  const showHostGrouping = (owner?.hosts.length ?? 0) > 1;
  const hasCards = agents.length > 0 || undeployedPersonas.length > 0;
  const groups = new Map<
    string,
    {
      label: string;
      agents: FleetProjectionOwnerPresentation["agents"][number][];
    }
  >();
  for (const agent of agents) {
    const host = owner ? fleetAgentHost(owner, agent.agentPubkey) : null;
    const key = showHostGrouping ? (host?.hostId ?? "unreported") : "fleet";
    const group = groups.get(key) ?? {
      label: host ? fleetHostLabel(host) : "Host not reported",
      agents: [],
    };
    group.agents.push(agent);
    groups.set(key, group);
  }

  return (
    <section className="relative space-y-4" data-testid="agents-fleet">
      {isLoading ? <FleetLoadingSkeleton /> : null}

      {!isLoading && !error && !hasCards ? (
        <div className="rounded-2xl border border-dashed border-border/80 px-5 py-6">
          <p className="font-medium">
            {emptyTitle ?? "No agents are being published"}
          </p>
          <p className="mt-1 text-sm text-muted-foreground">
            {emptyDescription ??
              "Agents appear here when a host publishes them."}
          </p>
        </div>
      ) : null}

      {!isLoading && hasCards && groups.size === 0 ? (
        <div
          className={IDENTITY_CARD_GRID_CLASS}
          data-testid="agents-fleet-grid"
        >
          {undeployedPersonas.map((persona) => (
            <ObserverUndeployedPersonaCard
              key={persona.personaId}
              persona={persona}
            />
          ))}
        </div>
      ) : null}

      {!isLoading && owner && groups.size > 0
        ? [...groups.entries()].map(([key, group], index, entries) => (
            <div className="space-y-3" key={key}>
              {showHostGrouping ? (
                <div className="flex items-center justify-between gap-3">
                  <h3 className="text-sm font-medium">{group.label}</h3>
                  <span className="text-xs text-muted-foreground">
                    {group.agents.length} agent
                    {group.agents.length === 1 ? "" : "s"}
                  </span>
                </div>
              ) : null}
              <div
                className={IDENTITY_CARD_GRID_CLASS}
                data-testid={
                  index === entries.length - 1 ? "agents-fleet-grid" : undefined
                }
              >
                {group.agents.map((agent) => (
                  <ProjectedFleetAgentCard
                    agent={agent}
                    isMessagePending={isMessagePending}
                    key={agent.agentPubkey}
                    now={now}
                    owner={owner}
                    showHost={showHostGrouping}
                    onMessageAgent={onMessageAgent}
                    onOpenAgentProfile={onOpenAgentProfile}
                  />
                ))}
                {index === entries.length - 1
                  ? undeployedPersonas.map((persona) => (
                      <ObserverUndeployedPersonaCard
                        key={persona.personaId}
                        persona={persona}
                      />
                    ))
                  : null}
              </div>
            </div>
          ))
        : null}

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
  getAvailability,
  isBestie,
  defaultModel,
  isActionPending,
  isMessagePending,
  isPersonaPending,
  persona,
  restartingAgentPubkey,
  startingAgentPubkey,
  onDeactivatePersona,
  onDeletePersona,
  onDuplicatePersona,
  onEditPersona,
  onMessageAgent,
  onOpenAgentProfile,
  onRestartAgent,
  onSharePersona,
  onSnapshotPersona,
  onStartAgent,
  onStopAgent,
}: {
  agent: ManagedAgent;
  getAvailability: AgentAvailabilityReader;
  isBestie: boolean;
  defaultModel: string;
  isActionPending: boolean;
  isMessagePending: boolean;
  isPersonaPending: boolean;
  persona?: AgentPersona;
  restartingAgentPubkey: string | null;
  startingAgentPubkey: string | null;
  onDeactivatePersona: (persona: AgentPersona) => void;
  onDeletePersona: (persona: AgentPersona) => void;
  onDuplicatePersona: (persona: AgentPersona) => void;
  /** `agent` is set when a deployed agent's card opens the editor. */
  onEditPersona: (persona: AgentPersona, agent?: ManagedAgent) => void;
  onMessageAgent: (pubkey: string) => void;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  onRestartAgent: (pubkey: string) => void;
  onSharePersona: (persona: AgentPersona) => void;
  onSnapshotPersona: (persona: AgentPersona) => void;
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
  const availability = getAvailability(agent.pubkey);
  const startBlockReason = agentPresenceStartBlockReason(
    isActive,
    availability,
  );
  const isRestarting = restartingAgentPubkey === agent.pubkey;
  const isStarting = startingAgentPubkey === agent.pubkey;
  const opensRuntimeTab = Boolean(friendlyError && !isActive);
  const status = hostFleetAgentStatus(agent);
  const runtimeLabel = resolveFleetAgentRuntimeLabel({ agent, persona });
  const runtimesQuery = useAcpRuntimesQuery();
  const modelLabel = resolveAgentCardModelLabel({
    agent,
    personaModel: persona?.model,
    defaultModel,
    effortModelQualifier: agentEffortModelQualifier(agent, runtimesQuery.data),
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
          <DropdownMenuContent align="end">
            <DropdownMenuItem
              disabled={isMessagePending}
              onSelect={() => onMessageAgent(agent.pubkey)}
            >
              <MessageSquare className="h-4 w-4" />
              Message
            </DropdownMenuItem>
            <DropdownMenuItem
              onSelect={() =>
                onOpenAgentProfile(agent.pubkey, { view: "activity" })
              }
            >
              <Activity className="h-4 w-4" />
              View activity
            </DropdownMenuItem>
            <DropdownMenuItem
              onSelect={() =>
                onOpenAgentProfile(agent.pubkey, { tab: "runtime" })
              }
            >
              <Settings2 className="h-4 w-4" />
              View runtime
            </DropdownMenuItem>
            {agent.backend.type === "local" && !agent.externalExecution ? (
              <DropdownMenuItem
                onSelect={() =>
                  onOpenAgentProfile(agent.pubkey, { view: "logs" })
                }
              >
                <TerminalSquare className="h-4 w-4" />
                View harness log
              </DropdownMenuItem>
            ) : null}
            <DropdownMenuSeparator />
            {agent.externalExecution ? (
              <DropdownMenuItem
                disabled={isActionPending}
                onSelect={() => onStartAgent(agent.pubkey)}
              >
                <Play className="h-4 w-4" />
                Open Codex Desktop
              </DropdownMenuItem>
            ) : isActive ? (
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
            ) : !startBlockReason ? (
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
            ) : null}
            {persona ? (
              <>
                <DropdownMenuSeparator />
                <PersonaActionItems
                  isActionPending={isActionPending}
                  isPending={isPersonaPending}
                  persona={persona}
                  onDeactivate={onDeactivatePersona}
                  onDelete={onDeletePersona}
                  onDuplicate={onDuplicatePersona}
                  onEdit={(edited) => onEditPersona(edited, agent)}
                  onShare={onSharePersona}
                  onSnapshot={onSnapshotPersona}
                />
              </>
            ) : null}
          </DropdownMenuContent>
        </DropdownMenu>
      }
      ariaLabel={`${identity.displayName} agent profile`}
      avatar={
        agent.externalExecution ? undefined : (
          <AgentRuntimeAvatarControl
            activeTestId={`agent-runtime-active-${agent.pubkey}`}
            avatarUrl={identity.avatarUrl}
            errorLabel={friendlyError}
            errorTestId={`agent-runtime-error-${agent.pubkey}`}
            isActive={isActive}
            availability={availability}
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
        )
      }
      avatarUrl={identity.avatarUrl}
      dataTestId={`managed-agent-${agent.pubkey}`}
      footerAccessory={
        <>
          <AgentManagementMarker pubkey={agent.pubkey} />
          <ProtectedBestieCardBadge agent={agent} isBestie={isBestie} />
        </>
      }
      label={identity.displayName}
      modelLabel={
        agent.externalExecution ? modelLabel : `${runtimeLabel} · ${modelLabel}`
      }
      subtitle={persona ? effectiveAgentDescription(persona) : null}
      onClick={() => {
        if (opensRuntimeTab) {
          onOpenAgentProfile(agent.pubkey, { tab: "runtime" });
        } else {
          onOpenAgentProfile(agent.pubkey);
        }
      }}
      statusBadge={
        <div className="flex flex-wrap gap-1.5">
          {agent.externalExecution ? (
            <Badge variant="outline">External app</Badge>
          ) : (
            <FleetAgentStatusChip
              status={status}
              testId={`managed-agent-status-${agent.pubkey}`}
            />
          )}
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
