import * as React from "react";
import {
  EllipsisVertical,
  OctagonX,
  Plus,
  Settings2,
  Upload,
} from "lucide-react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useBakedBuildEnvQuery } from "@/features/agents/hooks";
import { indexAgentDeployments } from "@/features/agents/ui/agentDeployments";
import { fleetAgentHost } from "@/features/agents/fleetProjection/selectors";
import type { FleetProjectionOwnerPresentation } from "@/features/agents/fleetProjection/useFleetProjectionPresentation";
import { useCurrentUnixSeconds } from "@/features/agents/fleetProjection/useCurrentUnixSeconds";
import { useFleetProjectionPresentation } from "@/features/agents/fleetProjection/useFleetProjectionPresentation";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { REMOTE_CONTROL_FEATURE_ID } from "@/features/agents/lib/remoteControlFeature";
import { useFleetVisibilitySettings } from "@/features/agents/lib/useFleetVisibilitySettings";
import { useGlobalAgentConfig } from "@/features/agents/useGlobalAgentConfig";
import { useOpenDmMutation } from "@/features/channels/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useProfilePanel } from "@/shared/context/ProfilePanelContext";
import { useFeatureEnabled } from "@/shared/features";
import { useFeedbackToasts } from "@/shared/hooks/useToastEffect";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { PageHeader } from "@/shared/ui/PageHeader";
import { AgentsDialogs } from "./AgentsDialogs";
import { FleetHostStatus } from "./FleetHostStatus";
import { FleetSection } from "./FleetSection";
import { fleetHostLabel } from "./fleetProjectionUi";
import { RemoteControlScopeSwitcher } from "./RemoteControlScopeSwitcher";
import { TeamsSection } from "./TeamsSection";
import { getInheritedAgentDefaults } from "./bakedEnvHelpers";
import { useManagedAgentActions } from "./useManagedAgentActions";
import { usePersonaActions } from "./usePersonaActions";
import { useTeamActions } from "./useTeamActions";

export function AgentsView() {
  const fleetVisibilityEnabled = useFeatureEnabled(REMOTE_CONTROL_FEATURE_ID);
  const fleetVisibility = useFleetVisibilitySettings();
  const identityQuery = useIdentityQuery();
  const fleetProjectionEnabled =
    fleetVisibilityEnabled && fleetVisibility?.enabled === true;
  const observerMode =
    fleetProjectionEnabled && fleetVisibility?.role === "observer";
  const needsRoleChoice =
    fleetProjectionEnabled && fleetVisibility?.needsChoice === true;

  if (fleetVisibilityEnabled && fleetVisibility === null) {
    return (
      <div className="flex-1 overflow-y-auto px-4 py-7 sm:px-6 sm:py-8">
        <div className="mx-auto w-full max-w-6xl space-y-8">
          <PageHeader
            description="Set up and manage your agents."
            title="Agents"
          />
          <p className="text-sm text-muted-foreground">
            Loading fleet settings…
          </p>
        </div>
      </div>
    );
  }

  return (
    <UnifiedAgentsView
      currentOwnerPubkey={identityQuery.data?.pubkey}
      fleetProjectionEnabled={fleetProjectionEnabled}
      localHostId={fleetVisibility?.hostId ?? null}
      needsRoleChoice={needsRoleChoice}
      observerMode={observerMode}
    />
  );
}

const LOCAL_SCOPE_ID = "local";
const ALL_HOSTS_SCOPE_ID = "all";

function UnifiedAgentsView({
  currentOwnerPubkey,
  fleetProjectionEnabled,
  localHostId,
  needsRoleChoice,
  observerMode,
}: {
  currentOwnerPubkey?: string;
  fleetProjectionEnabled: boolean;
  localHostId: string | null;
  needsRoleChoice: boolean;
  observerMode: boolean;
}) {
  const { openPersonaProfilePanel, openProfilePanel } = useProfilePanel();
  const { goChannel } = useAppNavigation();
  const openDmMutation = useOpenDmMutation();
  const { globalConfig } = useGlobalAgentConfig();
  const { data: bakedEnv } = useBakedBuildEnvQuery({ enabled: true });
  const inheritedDefaults = getInheritedAgentDefaults(globalConfig, bakedEnv);
  const agents = useManagedAgentActions();
  const personas = usePersonaActions();
  const snapshotImportInputRef = React.useRef<HTMLInputElement | null>(null);
  const aiDefaultsTriggerRef = React.useRef<HTMLButtonElement>(null);
  const actionsMenuTriggerRef = React.useRef<HTMLButtonElement>(null);
  const [isAiDefaultsOpen, setIsAiDefaultsOpen] = React.useState(false);
  const projection = useFleetProjectionPresentation(fleetProjectionEnabled);
  const now = useCurrentUnixSeconds(fleetProjectionEnabled);

  const teamActions = useTeamActions(
    {
      setActionNoticeMessage: agents.setActionNoticeMessage,
      setActionErrorMessage: agents.setActionErrorMessage,
    },
    {
      refetchManagedAgents: agents.refetchManagedAgents,
      refetchRelayAgents: agents.refetchRelayAgents,
    },
  );

  const personasById = React.useMemo(
    () =>
      new Map(
        (personas.personasQuery.data ?? []).map((persona) => [
          persona.id,
          persona,
        ]),
      ),
    [personas.personasQuery.data],
  );
  const deploymentsByPersonaId = React.useMemo(
    () => indexAgentDeployments(agents.managedAgents),
    [agents.managedAgents],
  );
  const [scopeId, setScopeId] = React.useState<string | null>(null);
  const isActionPending =
    agents.isPending ||
    personas.isPending ||
    teamActions.createTeamMutation.isPending ||
    teamActions.updateTeamMutation.isPending ||
    teamActions.deleteTeamMutation.isPending;
  const runningAgentCount = agents.managedAgents.filter((agent) =>
    isManagedAgentActive(agent),
  ).length;
  const hasSavedAgentDefaults = Boolean(
    globalConfig.preferred_runtime?.trim() ||
      globalConfig.provider?.trim() ||
      globalConfig.model?.trim() ||
      Object.values(globalConfig.env_vars).some(
        (value) => value.trim().length > 0,
      ),
  );
  const agentsError =
    agents.managedAgentsQuery.error instanceof Error
      ? agents.managedAgentsQuery.error
      : null;
  const personasError =
    personas.personasQuery.error instanceof Error
      ? personas.personasQuery.error
      : null;
  const teamsError =
    teamActions.teamsQuery.error instanceof Error
      ? teamActions.teamsQuery.error
      : null;
  const isAgentDataLoading =
    agents.managedAgentsQuery.isLoading || personas.personasQuery.isLoading;
  const normalizedOwnerPubkey = currentOwnerPubkey
    ? normalizePubkey(currentOwnerPubkey)
    : null;
  const currentFleetOwner =
    projection.owners.find(
      (owner) => owner.ownerPubkey === normalizedOwnerPubkey,
    ) ?? null;
  const projectedTeamOwners = fleetProjectionEnabled
    ? projection.owners.filter(
        (owner) =>
          owner.ownerPubkey !== normalizedOwnerPubkey &&
          owner.agents.length > 0,
      )
    : [];
  const projectionError = projection.errorMessage
    ? new Error(projection.errorMessage)
    : null;
  const remoteHosts = observerMode
    ? (currentFleetOwner?.hosts ?? [])
    : (currentFleetOwner?.hosts ?? []).filter(
        (host) => host.hostId !== localHostId,
      );
  const scopeOptions = [
    { id: LOCAL_SCOPE_ID, label: "This Mac" },
    ...(remoteHosts.length > 1
      ? [{ id: ALL_HOSTS_SCOPE_ID, label: "All hosts" }]
      : []),
    ...remoteHosts.map((host) => ({
      id: host.hostId,
      label: fleetHostLabel(host),
    })),
  ];
  const defaultScopeId = observerMode
    ? remoteHosts.length === 1
      ? remoteHosts[0].hostId
      : ALL_HOSTS_SCOPE_ID
    : LOCAL_SCOPE_ID;
  const resolvedScopeId = scopeId ?? defaultScopeId;
  const viewingRemote =
    fleetProjectionEnabled && resolvedScopeId !== LOCAL_SCOPE_ID;
  const selectedRemoteHost =
    viewingRemote && resolvedScopeId !== ALL_HOSTS_SCOPE_ID
      ? (remoteHosts.find((host) => host.hostId === resolvedScopeId) ??
        currentFleetOwner?.hosts.find(
          (host) => host.hostId === resolvedScopeId,
        ) ??
        null)
      : null;
  const scopedOwner = React.useMemo(() => {
    if (!currentFleetOwner) return null;
    if (!viewingRemote || resolvedScopeId === ALL_HOSTS_SCOPE_ID) {
      return currentFleetOwner;
    }
    return ownerForHost(currentFleetOwner, resolvedScopeId);
  }, [currentFleetOwner, resolvedScopeId, viewingRemote]);
  const canMutate = !observerMode && !needsRoleChoice && !viewingRemote;
  const undeployedHostPersonas = personas.libraryPersonas.filter(
    (persona) => (deploymentsByPersonaId.get(persona.id) ?? []).length === 0,
  );
  const deployedProjectedPersonaIds = new Set(
    (scopedOwner?.agents ?? [])
      .map((agent) => agent.personaId)
      .filter((personaId): personaId is string => Boolean(personaId)),
  );
  const undeployedProjectedPersonas = (scopedOwner?.personas ?? []).filter(
    (persona) => !deployedProjectedPersonaIds.has(persona.personaId),
  );
  const showLocalHostRoster = !observerMode && !viewingRemote;
  const showObserverLocalEmpty = observerMode && !viewingRemote;
  const showRemoteRoster = viewingRemote;
  const showOnboarding =
    showLocalHostRoster &&
    !needsRoleChoice &&
    !isAgentDataLoading &&
    !agentsError &&
    !personasError &&
    agents.managedAgents.length === 0 &&
    personas.libraryPersonas.length === 0;

  useFeedbackToasts(agents.actionNoticeMessage, agents.actionErrorMessage);
  useFeedbackToasts(
    personas.personaFeedbackSurface === "library"
      ? personas.personaNoticeMessage
      : null,
    personas.personaFeedbackSurface === "library"
      ? personas.personaErrorMessage
      : null,
  );

  function openUnifiedCatalog() {
    personas.prepareCreate();
    personas.openCatalog();
  }

  function openAiDefaults(trigger: HTMLButtonElement | null) {
    aiDefaultsTriggerRef.current = trigger;
    setIsAiDefaultsOpen(true);
  }

  function setAiDefaultsDialogOpen(open: boolean) {
    if (!open) aiDefaultsTriggerRef.current = actionsMenuTriggerRef.current;
    setIsAiDefaultsOpen(open);
  }

  const handleMessageAgent = React.useCallback(
    async (pubkey: string) => {
      const dm = await openDmMutation.mutateAsync({ pubkeys: [pubkey] });
      await goChannel(dm.id);
    },
    [goChannel, openDmMutation],
  );

  return (
    <>
      <div className="flex-1 overflow-y-auto overflow-x-hidden overscroll-contain px-4 py-7 sm:px-6 sm:py-8">
        <div
          className="mx-auto w-full max-w-6xl space-y-8 [container-type:inline-size]"
          data-testid="agents-page-content"
        >
          <PageHeader
            action={
              canMutate ? (
                <div className="flex items-center gap-2">
                  <Button
                    data-testid="new-agent-card"
                    disabled={personas.isPending}
                    onClick={openUnifiedCatalog}
                    size="sm"
                    type="button"
                  >
                    <Plus className="h-4 w-4" />
                    New agent
                  </Button>
                  <DropdownMenu modal={false}>
                    <DropdownMenuTrigger asChild>
                      <Button
                        aria-label="Agent actions"
                        data-testid="agent-actions-menu-trigger"
                        ref={actionsMenuTriggerRef}
                        size="icon"
                        type="button"
                        variant="outline"
                      >
                        <EllipsisVertical />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      <DropdownMenuItem
                        onSelect={() =>
                          openAiDefaults(actionsMenuTriggerRef.current)
                        }
                      >
                        <Settings2 />
                        {hasSavedAgentDefaults
                          ? "Agent defaults"
                          : "Set agent defaults"}
                      </DropdownMenuItem>
                      {runningAgentCount > 0 ? (
                        <DropdownMenuItem
                          disabled={isActionPending}
                          onSelect={() => {
                            void agents.handleBulkStopRunning();
                          }}
                        >
                          <OctagonX />
                          Stop running agents
                        </DropdownMenuItem>
                      ) : null}
                      <DropdownMenuSeparator />
                      <DropdownMenuItem
                        disabled={isActionPending}
                        onSelect={() => snapshotImportInputRef.current?.click()}
                      >
                        <Upload />
                        Import snapshot
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </div>
              ) : null
            }
            description={
              needsRoleChoice
                ? "Choose this device's role before viewing agents."
                : viewingRemote
                  ? selectedRemoteHost?.name?.trim()
                    ? `Showing agents on ${selectedRemoteHost.name.trim()}.`
                    : "Showing agents on another Mac."
                  : observerMode
                    ? "No agents run on this Mac. Switch to a host to see the fleet."
                    : "Agents run on this Mac."
            }
            title="Agents"
          />

          {fleetProjectionEnabled && !needsRoleChoice ? (
            <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              {currentOwnerPubkey ? (
                <FleetHostStatus
                  hosts={currentFleetOwner?.hosts ?? []}
                  now={now}
                />
              ) : (
                <p className="text-sm text-muted-foreground">
                  Loading fleet identity…
                </p>
              )}
              {scopeOptions.length > 1 ? (
                <RemoteControlScopeSwitcher
                  options={scopeOptions}
                  value={resolvedScopeId}
                  onChange={setScopeId}
                />
              ) : null}
            </div>
          ) : null}

          {needsRoleChoice ? (
            <FleetRoleChoiceBanner />
          ) : showOnboarding ? (
            <AgentsOnboardingState onCreate={openUnifiedCatalog} />
          ) : (
            <div className="flex flex-col gap-8">
              {showLocalHostRoster ? (
                <FleetSection
                  agents={agents.managedAgents}
                  defaultModel={inheritedDefaults.model.value}
                  error={agentsError ?? personasError}
                  isActionPending={isActionPending}
                  isLoading={
                    agents.managedAgentsQuery.isLoading ||
                    personas.personasQuery.isLoading
                  }
                  isMessagePending={openDmMutation.isPending}
                  isPersonaPending={personas.isPending}
                  mode="host"
                  personasById={personasById}
                  restartingAgentPubkey={agents.restartingAgentPubkey}
                  startingAgentPubkey={agents.startingAgentPubkey}
                  startingPersonaIds={agents.startingPersonaIds}
                  undeployedPersonas={undeployedHostPersonas}
                  onDeactivatePersona={(persona) => {
                    void personas.handleSetActive(persona, false, "library");
                  }}
                  onDeletePersona={personas.openDelete}
                  onDeployPersona={(persona) => {
                    void agents.handleStartPersona(persona);
                  }}
                  onDuplicatePersona={personas.openDuplicate}
                  onEditPersona={personas.openEdit}
                  onMessageAgent={(pubkey) => {
                    void handleMessageAgent(pubkey);
                  }}
                  onOpenAgentProfile={(pubkey, options) =>
                    openProfilePanel?.(pubkey, options)
                  }
                  onOpenPersonaProfile={(persona) =>
                    openPersonaProfilePanel?.(persona)
                  }
                  onRestartAgent={(pubkey) => {
                    void agents.handleRestart(pubkey);
                  }}
                  onSharePersona={(persona) => {
                    const deployments =
                      deploymentsByPersonaId.get(persona.id) ?? [];
                    personas.openShare(persona, deployments, persona.avatarUrl);
                  }}
                  onSnapshotPersona={(persona) => {
                    const deployments =
                      deploymentsByPersonaId.get(persona.id) ?? [];
                    personas.openExportSnapshot(
                      persona,
                      deployments,
                      persona.avatarUrl,
                    );
                  }}
                  onStartAgent={(pubkey) => {
                    void agents.handleStart(pubkey);
                  }}
                  onStopAgent={(pubkey) => {
                    void agents.handleStop(pubkey);
                  }}
                />
              ) : (
                <FleetSection
                  emptyDescription={
                    showObserverLocalEmpty
                      ? "This Mac is an Observer, so it does not run agents. Switch to a host to see the fleet."
                      : undefined
                  }
                  emptyTitle={
                    showObserverLocalEmpty ? "No agents on this Mac" : undefined
                  }
                  error={projectionError}
                  isLoading={projection.loading || !currentOwnerPubkey}
                  isMessagePending={openDmMutation.isPending}
                  mode="observer"
                  now={now}
                  owner={
                    showRemoteRoster
                      ? scopedOwner
                      : showObserverLocalEmpty
                        ? null
                        : currentFleetOwner
                  }
                  undeployedPersonas={
                    showRemoteRoster ? undeployedProjectedPersonas : []
                  }
                  onMessageAgent={(pubkey) => {
                    void handleMessageAgent(pubkey);
                  }}
                  onOpenAgentProfile={(pubkey, options) =>
                    openProfilePanel?.(pubkey, {
                      ...options,
                      source: "relay",
                    })
                  }
                />
              )}

              {!showLocalHostRoster ? (
                <TeamsSection
                  error={projectionError}
                  isLoading={projection.loading || !currentOwnerPubkey}
                  isMessagePending={openDmMutation.isPending}
                  mode="observer"
                  now={now}
                  projectedOwners={
                    showObserverLocalEmpty ? [] : projectedTeamOwners
                  }
                  onMessageAgent={(pubkey) => {
                    void handleMessageAgent(pubkey);
                  }}
                  onOpenAgentProfile={(pubkey, options) =>
                    openProfilePanel?.(pubkey, options)
                  }
                />
              ) : (
                <TeamsSection
                  error={teamsError}
                  isLoading={teamActions.teamsQuery.isLoading}
                  isMessagePending={openDmMutation.isPending}
                  isPending={
                    teamActions.createTeamMutation.isPending ||
                    teamActions.updateTeamMutation.isPending ||
                    teamActions.deleteTeamMutation.isPending
                  }
                  mode="host"
                  now={now}
                  projectedOwners={projectedTeamOwners}
                  onAddToChannel={teamActions.setTeamToAddToChannel}
                  onCreate={teamActions.openCreateDialog}
                  onDelete={teamActions.setTeamToDelete}
                  onDuplicate={teamActions.openDuplicateDialog}
                  onEdit={teamActions.openEditDialog}
                  onImport={() => snapshotImportInputRef.current?.click()}
                  onMessageAgent={(pubkey) => {
                    void handleMessageAgent(pubkey);
                  }}
                  onOpenAgentProfile={(pubkey, options) =>
                    openProfilePanel?.(pubkey, options)
                  }
                  onShare={teamActions.openShare}
                  personas={personas.libraryPersonas}
                  teams={teamActions.teams}
                />
              )}
            </div>
          )}
        </div>
      </div>

      {canMutate ? (
        <AgentsDialogs
          agents={agents}
          aiDefaultsOpen={isAiDefaultsOpen}
          aiDefaultsReturnFocusRef={aiDefaultsTriggerRef}
          onAiDefaultsOpenChange={setAiDefaultsDialogOpen}
          onOpenPersonaProfile={(persona) => openPersonaProfilePanel?.(persona)}
          personas={personas}
          snapshotImportInputRef={snapshotImportInputRef}
          teamActions={teamActions}
        />
      ) : null}
    </>
  );
}

function AgentsOnboardingState({ onCreate }: { onCreate: () => void }) {
  return (
    <section
      className="rounded-3xl border border-dashed border-border/80 bg-muted/20 px-6 py-14 text-center"
      data-testid="agents-empty-state"
    >
      <div className="mx-auto max-w-md">
        <h2 className="text-xl font-semibold">Create your first agent</h2>
        <p className="mt-2 text-sm text-muted-foreground">
          Start with a template, then deploy it as an agent on this device.
        </p>
        <Button className="mt-5" onClick={onCreate} type="button">
          <Plus className="h-4 w-4" />
          New agent
        </Button>
      </div>
    </section>
  );
}

function ownerForHost(
  owner: FleetProjectionOwnerPresentation,
  hostId: string,
): FleetProjectionOwnerPresentation {
  const hosts = owner.hosts.filter((host) => host.hostId === hostId);
  const agents = owner.agents.filter(
    (agent) => fleetAgentHost(owner, agent.agentPubkey)?.hostId === hostId,
  );
  const deployedPersonaIds = new Set(
    agents
      .map((agent) => agent.personaId)
      .filter((personaId): personaId is string => Boolean(personaId)),
  );
  const personas = owner.personas.filter(
    (persona) =>
      deployedPersonaIds.has(persona.personaId) ||
      !owner.agents.some((agent) => agent.personaId === persona.personaId),
  );
  return { ...owner, agents, hosts, personas };
}

function FleetRoleChoiceBanner() {
  return (
    <section
      className="rounded-2xl border border-amber-500/30 bg-amber-500/10 px-5 py-5"
      data-testid="fleet-role-choice-banner"
    >
      <h2 className="font-semibold">Choose this device's fleet role</h2>
      <p className="mt-1 text-sm text-muted-foreground">
        This device has existing local agent data. Choose Host or Observer in
        the role chooser before the roster is shown, so stale local agents are
        never presented as the current fleet.
      </p>
    </section>
  );
}
