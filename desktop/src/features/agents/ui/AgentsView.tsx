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
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { useFleetVisibilitySettings } from "@/features/agents/lib/useFleetVisibilitySettings";
import { useGlobalAgentConfig } from "@/features/agents/useGlobalAgentConfig";
import { useOpenDmMutation } from "@/features/channels/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useProfilePanel } from "@/shared/context/ProfilePanelContext";
import { useFeatureEnabled } from "@/shared/features";
import { useFeedbackToasts } from "@/shared/hooks/useToastEffect";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { PageHeader } from "@/shared/ui/PageHeader";
import { AgentLibrarySection } from "./AgentLibrarySection";
import { AgentsDialogs } from "./AgentsDialogs";
import { FleetSection } from "./FleetSection";
import {
  FleetTeamSection,
  ObserverFleetSections,
} from "./FleetProjectionSections";
import { TeamsSection } from "./TeamsSection";
import { getInheritedAgentDefaults } from "./bakedEnvHelpers";
import { useManagedAgentActions } from "./useManagedAgentActions";
import { usePersonaActions } from "./usePersonaActions";
import { useTeamActions } from "./useTeamActions";

export function AgentsView() {
  const fleetVisibilityEnabled = useFeatureEnabled("fleetVisibility");
  const fleetVisibility = useFleetVisibilitySettings();
  const identityQuery = useIdentityQuery();
  const fleetProjectionEnabled =
    fleetVisibilityEnabled && fleetVisibility?.enabled === true;
  const observerMode =
    fleetProjectionEnabled && fleetVisibility?.role === "observer";

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

  if (observerMode) {
    const currentOwnerPubkey = identityQuery.data?.pubkey;
    return (
      <div className="flex-1 overflow-y-auto overflow-x-hidden overscroll-contain px-4 py-7 sm:px-6 sm:py-8">
        <div
          className="mx-auto w-full max-w-6xl space-y-8 [container-type:inline-size]"
          data-testid="agents-page-content"
        >
          <PageHeader
            description="View and message the fleet hosted on your other device."
            title="Agents"
          />
          {currentOwnerPubkey ? (
            <ObserverFleetSections currentOwnerPubkey={currentOwnerPubkey} />
          ) : (
            <p className="text-sm text-muted-foreground">
              Loading fleet identity…
            </p>
          )}
        </div>
      </div>
    );
  }

  return (
    <HostAgentsView
      currentOwnerPubkey={identityQuery.data?.pubkey}
      showFleetTeam={fleetProjectionEnabled}
    />
  );
}

function HostAgentsView({
  currentOwnerPubkey,
  showFleetTeam,
}: {
  currentOwnerPubkey?: string;
  showFleetTeam: boolean;
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
  const showOnboarding =
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
            }
            description="Agents run on this device."
            title="Agents"
          />

          {showOnboarding ? (
            <AgentsOnboardingState onCreate={openUnifiedCatalog} />
          ) : (
            <div className="flex flex-col gap-8">
              <FleetSection
                agents={agents.managedAgents}
                defaultModel={inheritedDefaults.model.value}
                error={agentsError}
                isActionPending={isActionPending}
                isLoading={agents.managedAgentsQuery.isLoading}
                isMessagePending={openDmMutation.isPending}
                personasById={personasById}
                restartingAgentPubkey={agents.restartingAgentPubkey}
                startingAgentPubkey={agents.startingAgentPubkey}
                onMessageAgent={(pubkey) => {
                  void handleMessageAgent(pubkey);
                }}
                onOpenAgentProfile={(pubkey, options) =>
                  openProfilePanel?.(pubkey, options)
                }
                onRestartAgent={(pubkey) => {
                  void agents.handleRestart(pubkey);
                }}
                onStartAgent={(pubkey) => {
                  void agents.handleStart(pubkey);
                }}
                onStopAgent={(pubkey) => {
                  void agents.handleStop(pubkey);
                }}
              />

              <AgentLibrarySection
                defaultModel={inheritedDefaults.model.value}
                deploymentsByPersonaId={deploymentsByPersonaId}
                error={personasError}
                isActionPending={isActionPending}
                isLoading={personas.personasQuery.isLoading}
                isPending={personas.isPending}
                personas={personas.libraryPersonas}
                startingPersonaIds={agents.startingPersonaIds}
                onBrowseCatalog={openUnifiedCatalog}
                onDeactivatePersona={(persona) => {
                  void personas.handleSetActive(persona, false, "library");
                }}
                onDeletePersona={personas.openDelete}
                onDeployPersona={(persona) => {
                  void agents.handleStartPersona(persona);
                }}
                onDuplicatePersona={personas.openDuplicate}
                onEditPersona={personas.openEdit}
                onOpenAgentProfile={(pubkey, options) =>
                  openProfilePanel?.(pubkey, options)
                }
                onOpenPersonaProfile={(persona) =>
                  openPersonaProfilePanel?.(persona)
                }
                onSharePersona={(persona, deployments) =>
                  personas.openShare(persona, deployments, persona.avatarUrl)
                }
                onSnapshotPersona={(persona, deployments) =>
                  personas.openExportSnapshot(
                    persona,
                    deployments,
                    persona.avatarUrl,
                  )
                }
              />

              <TeamsSection
                error={teamsError}
                isLoading={teamActions.teamsQuery.isLoading}
                isPending={
                  teamActions.createTeamMutation.isPending ||
                  teamActions.updateTeamMutation.isPending ||
                  teamActions.deleteTeamMutation.isPending
                }
                onAddToChannel={teamActions.setTeamToAddToChannel}
                onCreate={teamActions.openCreateDialog}
                onDelete={teamActions.setTeamToDelete}
                onDuplicate={teamActions.openDuplicateDialog}
                onEdit={teamActions.openEditDialog}
                onImport={() => snapshotImportInputRef.current?.click()}
                onShare={teamActions.openShare}
                personas={personas.libraryPersonas}
                teams={teamActions.teams}
              />

              {showFleetTeam && currentOwnerPubkey ? (
                <FleetTeamSection currentOwnerPubkey={currentOwnerPubkey} />
              ) : null}
            </div>
          )}
        </div>
      </div>

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
