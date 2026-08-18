import {
  CopyPlus,
  EllipsisVertical,
  Pencil,
  Rocket,
  Share2,
  Trash2,
} from "lucide-react";

import { resolveTeamPersonas } from "@/features/agents/lib/teamPersonas";
import type { FleetProjectionOwnerPresentation } from "@/features/agents/fleetProjection/useFleetProjectionPresentation";
import type { AgentPersona, AgentTeam } from "@/shared/api/types";
import type { ProfilePanelOpenOptions } from "@/shared/context/ProfilePanelContext";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { IdentityCardSkeleton } from "@/shared/ui/identity-card-skeleton";
import { SectionHeader } from "@/shared/ui/PageHeader";
import { CreateIdentityCard } from "./CreateIdentityCard";
import { TeamIdentityCard } from "./TeamIdentityCard";
import { IDENTITY_CARD_GRID_CLASS } from "./agentCardLayout";
import { ProjectedFleetAgentCard } from "./ProjectedFleetAgentCard";

const TEAM_CARD_COLUMN_CLASS = "w-full";

type ProjectedTeamProps = {
  isMessagePending: boolean;
  now: number;
  projectedOwners: readonly FleetProjectionOwnerPresentation[];
  onMessageAgent: (pubkey: string) => void;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
};

type HostTeamsSectionProps = ProjectedTeamProps & {
  mode: "host";
  teams: AgentTeam[];
  personas: AgentPersona[];
  error: Error | null;
  isLoading: boolean;
  isPending: boolean;
  onCreate: () => void;
  onDuplicate: (team: AgentTeam) => void;
  onEdit: (team: AgentTeam) => void;
  onDelete: (team: AgentTeam) => void;
  onAddToChannel: (team: AgentTeam) => void;
  onShare: (team: AgentTeam) => void;
  onImport: () => void;
};

type ObserverTeamsSectionProps = ProjectedTeamProps & {
  mode: "observer";
  error: Error | null;
  isLoading: boolean;
};

type TeamsSectionProps = HostTeamsSectionProps | ObserverTeamsSectionProps;

export function TeamsSection(props: TeamsSectionProps) {
  if (props.mode === "observer") return <ObserverTeamsSection {...props} />;
  const {
    teams,
    personas,
    error,
    isLoading,
    isPending,
    projectedOwners,
    now,
    isMessagePending,
    onCreate,
    onDuplicate,
    onEdit,
    onDelete,
    onAddToChannel,
    onShare,
    onImport,
    onMessageAgent,
    onOpenAgentProfile,
  } = props;
  return (
    <section className="relative space-y-4" data-testid="agents-library-teams">
      <div className={TEAM_CARD_COLUMN_CLASS}>
        <SectionHeader
          title="Teams"
          description="Deploy groups of agents to a channel together."
        />
      </div>

      {isLoading ? (
        <div className={IDENTITY_CARD_GRID_CLASS}>
          <IdentityCardSkeleton
            footerSubtitleWidthClass="w-14"
            footerTitleWidthClass="w-24"
            showAction
          />
          <IdentityCardSkeleton
            footerSubtitleWidthClass="w-24"
            footerTitleWidthClass="w-32"
            showAction
          />
          <IdentityCardSkeleton
            footerSubtitleWidthClass="w-20"
            footerTitleWidthClass="w-28"
            showAction
          />
        </div>
      ) : null}

      {!isLoading ? (
        <div className={IDENTITY_CARD_GRID_CLASS}>
          <NewTeamCard
            isPending={isPending}
            onCreate={onCreate}
            onImport={onImport}
          />
          {teams.map((team) => {
            const resolution = resolveTeamPersonas(team, personas);
            const missingPersonaCount = resolution.missingPersonaCount;
            const hasMissingPersonas = resolution.hasMissingPersonas;

            return (
              <TeamIdentityCard
                actions={
                  <DropdownMenu modal={false}>
                    <DropdownMenuTrigger asChild>
                      <button
                        aria-label={`${team.name} team actions`}
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
                        disabled={isPending || hasMissingPersonas}
                        onClick={() => onAddToChannel(team)}
                      >
                        <Rocket className="h-4 w-4" />
                        Deploy to channel
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem
                        disabled={isPending}
                        onClick={() => onEdit(team)}
                      >
                        <Pencil className="h-4 w-4" />
                        Edit
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        disabled={isPending || hasMissingPersonas}
                        onClick={() => onDuplicate(team)}
                      >
                        <CopyPlus className="h-4 w-4" />
                        Duplicate
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        disabled={isPending || hasMissingPersonas}
                        onClick={() => onShare(team)}
                      >
                        <Share2 className="h-4 w-4" />
                        Share
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem
                        className="text-destructive focus:text-destructive"
                        disabled={isPending}
                        onClick={() => onDelete(team)}
                      >
                        <Trash2 className="h-4 w-4" />
                        Delete
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                }
                dataTestId={`team-card-${team.id}`}
                description={team.description}
                isSymlink={team.isSymlink}
                key={team.id}
                memberCount={team.personaIds.length}
                personas={resolution.resolvedPersonas}
                sourceDir={team.sourceDir}
                symlinkTarget={team.symlinkTarget}
                teamName={team.name}
                version={team.version}
              >
                {hasMissingPersonas ? (
                  <p className="border-t border-destructive/20 bg-destructive/10 px-3 py-2 text-xs text-destructive">
                    {missingPersonaCount} agent
                    {missingPersonaCount === 1 ? "" : "s"} in this team{" "}
                    {missingPersonaCount === 1 ? "is" : "are"} no longer in your
                    agents. Edit the team to fix it before deploying or sharing.
                  </p>
                ) : null}
              </TeamIdentityCard>
            );
          })}
        </div>
      ) : null}

      <ProjectedTeamRosters
        isMessagePending={isMessagePending}
        now={now}
        owners={projectedOwners}
        onMessageAgent={onMessageAgent}
        onOpenAgentProfile={onOpenAgentProfile}
      />

      {error ? (
        <p
          className={`${TEAM_CARD_COLUMN_CLASS} rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive`}
        >
          {error.message}
        </p>
      ) : null}
    </section>
  );
}

function ObserverTeamsSection({
  error,
  isLoading,
  isMessagePending,
  now,
  projectedOwners,
  onMessageAgent,
  onOpenAgentProfile,
}: ObserverTeamsSectionProps) {
  return (
    <section className="relative space-y-4" data-testid="agents-library-teams">
      <SectionHeader
        description="Read-only fleets from people who share a channel with you."
        title="Teams"
      />
      {isLoading ? (
        <div className={IDENTITY_CARD_GRID_CLASS}>
          <IdentityCardSkeleton
            footerSubtitleWidthClass="w-20"
            footerTitleWidthClass="w-28"
          />
        </div>
      ) : null}
      {!isLoading && projectedOwners.length === 0 && !error ? (
        <div className="rounded-2xl border border-dashed border-border/80 px-5 py-6">
          <p className="font-medium">No shared fleet teams yet</p>
          <p className="mt-1 text-sm text-muted-foreground">
            Fleets from channel members appear here when their hosts publish
            them.
          </p>
        </div>
      ) : null}
      {!isLoading ? (
        <ProjectedTeamRosters
          isMessagePending={isMessagePending}
          now={now}
          owners={projectedOwners}
          onMessageAgent={onMessageAgent}
          onOpenAgentProfile={onOpenAgentProfile}
        />
      ) : null}
      {error ? (
        <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
          {error.message}
        </p>
      ) : null}
    </section>
  );
}

function ProjectedTeamRosters({
  isMessagePending,
  now,
  owners,
  onMessageAgent,
  onOpenAgentProfile,
}: Omit<ProjectedTeamProps, "projectedOwners"> & {
  owners: readonly FleetProjectionOwnerPresentation[];
}) {
  if (owners.length === 0) return null;
  return (
    <div className="space-y-5" data-testid="fleet-team-rosters">
      {owners.map((owner) => (
        <div className="space-y-3" key={owner.ownerPubkey}>
          <div className="flex items-center justify-between gap-3">
            <div>
              <h3 className="text-sm font-medium">{owner.displayName}</h3>
              <p className="text-xs text-muted-foreground">
                {owner.agents.length} agent
                {owner.agents.length === 1 ? "" : "s"}
              </p>
            </div>
            <span className="rounded-full border border-border/70 px-2 py-1 text-2xs font-medium text-muted-foreground">
              Read-only
            </span>
          </div>
          <div className={IDENTITY_CARD_GRID_CLASS}>
            {owner.agents.map((agent) => (
              <ProjectedFleetAgentCard
                agent={agent}
                isMessagePending={isMessagePending}
                key={agent.agentPubkey}
                now={now}
                owner={owner}
                showHost={owner.hosts.length > 1}
                onMessageAgent={onMessageAgent}
                onOpenAgentProfile={onOpenAgentProfile}
              />
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

function NewTeamCard({
  isPending,
  onCreate,
  onImport,
}: {
  isPending: boolean;
  onCreate: () => void;
  onImport: () => void;
}) {
  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <CreateIdentityCard ariaLabel="New team" dataTestId="new-team-card" />
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        onCloseAutoFocus={(event) => event.preventDefault()}
      >
        <DropdownMenuItem disabled={isPending} onClick={onCreate}>
          Create team
        </DropdownMenuItem>
        <DropdownMenuItem disabled={isPending} onClick={onImport}>
          Import
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
