import { Library, Rocket } from "lucide-react";

import { resolveAgentCardModelLabel } from "@/features/agents/lib/agentCardModelLabel";
import type { FleetProjectionPersona } from "@/features/agents/fleetProjection/types";
import type { FleetProjectionAgentPresentation } from "@/features/agents/fleetProjection/selectors";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { AgentPersona } from "@/shared/api/types";
import type { ProfilePanelOpenOptions } from "@/shared/context/ProfilePanelContext";
import { Button } from "@/shared/ui/button";
import { SectionHeader } from "@/shared/ui/PageHeader";
import { Skeleton } from "@/shared/ui/skeleton";
import type { AgentDeployment } from "./agentDeployments";
import { IdentityInitialsAvatar } from "./IdentityInitialsAvatar";
import { PersonaActionsMenu } from "./PersonaActionsMenu";

type HostAgentLibrarySectionProps = {
  mode: "host";
  defaultModel: string;
  deploymentsByPersonaId: ReadonlyMap<string, AgentDeployment[]>;
  error: Error | null;
  isActionPending: boolean;
  isLoading: boolean;
  isPending: boolean;
  personas: AgentPersona[];
  startingPersonaIds: ReadonlySet<string>;
  onBrowseCatalog: () => void;
  onDeactivatePersona: (persona: AgentPersona) => void;
  onDeletePersona: (persona: AgentPersona) => void;
  onDeployPersona: (persona: AgentPersona) => void;
  onDuplicatePersona: (persona: AgentPersona) => void;
  onEditPersona: (persona: AgentPersona) => void;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  onOpenPersonaProfile: (persona: AgentPersona) => void;
  onSharePersona: (
    persona: AgentPersona,
    deployments: AgentDeployment[],
  ) => void;
  onSnapshotPersona: (
    persona: AgentPersona,
    deployments: AgentDeployment[],
  ) => void;
};

type ObserverAgentLibrarySectionProps = {
  mode: "observer";
  agents: readonly FleetProjectionAgentPresentation[];
  error: Error | null;
  isLoading: boolean;
  personas: readonly FleetProjectionPersona[];
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
};

type AgentLibrarySectionProps =
  | HostAgentLibrarySectionProps
  | ObserverAgentLibrarySectionProps;

type LibraryCardModel = {
  avatarUrl: string | null;
  deployments: AgentDeployment[];
  displayName: string;
  id: string;
  modelLabel: string;
  persona?: AgentPersona;
  readOnly: boolean;
  runtimeLabel: string;
};

export function AgentLibrarySection(props: AgentLibrarySectionProps) {
  const isHost = props.mode === "host";
  const cards: LibraryCardModel[] = isHost
    ? props.personas.map((persona) => ({
        avatarUrl: persona.avatarUrl?.trim() || null,
        deployments: props.deploymentsByPersonaId.get(persona.id) ?? [],
        displayName: persona.displayName,
        id: persona.id,
        modelLabel: resolveAgentCardModelLabel({
          agent: undefined,
          personaModel: persona.model,
          defaultModel: props.defaultModel,
        }),
        persona,
        readOnly: false,
        runtimeLabel: persona.runtime?.trim() || "Default runtime",
      }))
    : props.personas.map((persona) => ({
        avatarUrl: persona.avatarUrl,
        deployments: props.agents
          .filter((agent) => agent.personaId === persona.personaId)
          .map((agent) => ({
            name: agent.displayName,
            pubkey: agent.agentPubkey,
          })),
        displayName: persona.displayName,
        id: persona.personaId,
        modelLabel: persona.model?.trim() || "Default model",
        readOnly: true,
        runtimeLabel: persona.runtime?.trim() || "Default runtime",
      }));
  const { error, isLoading } = props;
  const isPending = isHost ? props.isPending : false;

  return (
    <section
      className="relative space-y-4"
      data-testid="agents-library-personas"
    >
      <SectionHeader
        action={
          isHost ? (
            <Button
              disabled={isPending}
              onClick={props.onBrowseCatalog}
              size="sm"
              type="button"
              variant="outline"
            >
              <Library className="h-4 w-4" />
              Browse catalog
            </Button>
          ) : null
        }
        description={
          isHost
            ? "Reusable templates for deploying agents into your fleet."
            : "Definitions shared by your fleet host."
        }
        title="Library"
      />

      {isLoading ? <LibraryLoadingSkeleton /> : null}

      {!isLoading && !error && cards.length === 0 ? (
        <div className="rounded-2xl border border-dashed border-border/80 px-5 py-6">
          <p className="font-medium">
            {isHost ? "No agent templates yet" : "No shared definitions yet"}
          </p>
          <p className="mt-1 text-sm text-muted-foreground">
            {isHost
              ? "Create a template or add one from the catalog."
              : "Definitions appear here when a fleet host shares them."}
          </p>
          {isHost ? (
            <Button
              className="mt-4"
              disabled={isPending}
              onClick={props.onBrowseCatalog}
              size="sm"
              type="button"
            >
              Browse catalog
            </Button>
          ) : null}
        </div>
      ) : null}

      {!isLoading && cards.length > 0 ? (
        <div
          className="grid grid-cols-1 gap-3 [@container(min-width:38rem)]:grid-cols-2"
          data-testid="agent-library-grid"
        >
          {cards.map((card) => {
            const persona = card.persona;
            return (
              <LibraryTemplateCard
                card={card}
                isActionPending={isHost ? props.isActionPending : false}
                isPending={isPending}
                isStarting={
                  isHost && persona
                    ? props.startingPersonaIds.has(persona.id)
                    : false
                }
                key={card.id}
                onDeactivatePersona={
                  isHost ? props.onDeactivatePersona : undefined
                }
                onDeletePersona={isHost ? props.onDeletePersona : undefined}
                onDeployPersona={isHost ? props.onDeployPersona : undefined}
                onDuplicatePersona={
                  isHost ? props.onDuplicatePersona : undefined
                }
                onEditPersona={isHost ? props.onEditPersona : undefined}
                onOpenAgentProfile={props.onOpenAgentProfile}
                onOpenPersonaProfile={
                  isHost ? props.onOpenPersonaProfile : undefined
                }
                onSharePersona={
                  isHost && persona
                    ? () => props.onSharePersona(persona, card.deployments)
                    : undefined
                }
                onSnapshotPersona={
                  isHost && persona
                    ? () => props.onSnapshotPersona(persona, card.deployments)
                    : undefined
                }
              />
            );
          })}
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

function LibraryTemplateCard({
  card,
  isActionPending,
  isPending,
  isStarting,
  onDeactivatePersona,
  onDeletePersona,
  onDeployPersona,
  onDuplicatePersona,
  onEditPersona,
  onOpenAgentProfile,
  onOpenPersonaProfile,
  onSharePersona,
  onSnapshotPersona,
}: {
  card: LibraryCardModel;
  isActionPending: boolean;
  isPending: boolean;
  isStarting: boolean;
  onDeactivatePersona?: (persona: AgentPersona) => void;
  onDeletePersona?: (persona: AgentPersona) => void;
  onDeployPersona?: (persona: AgentPersona) => void;
  onDuplicatePersona?: (persona: AgentPersona) => void;
  onEditPersona?: (persona: AgentPersona) => void;
  onOpenAgentProfile: (pubkey: string) => void;
  onOpenPersonaProfile?: (persona: AgentPersona) => void;
  onSharePersona?: () => void;
  onSnapshotPersona?: () => void;
}) {
  const { deployments, persona } = card;
  const mutationActions =
    persona &&
    onDeactivatePersona &&
    onDeletePersona &&
    onDeployPersona &&
    onDuplicatePersona &&
    onEditPersona &&
    onSharePersona &&
    onSnapshotPersona
      ? {
          onDeactivatePersona,
          onDeletePersona,
          onDeployPersona,
          onDuplicatePersona,
          onEditPersona,
          onSharePersona,
          onSnapshotPersona,
          persona,
        }
      : null;
  const identity = (
    <>
      {card.avatarUrl?.trim() ? (
        <ProfileAvatar
          avatarUrl={card.avatarUrl}
          className="h-12 w-12 shrink-0 border-2 border-background bg-muted shadow-none"
          iconClassName="h-5 w-5"
          label={card.displayName}
        />
      ) : (
        <IdentityInitialsAvatar
          className="h-12 w-12 shrink-0 border-0 shadow-none"
          label={card.displayName}
          size={48}
        />
      )}
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-semibold">
          {card.displayName}
        </span>
        <span className="mt-0.5 block truncate text-xs text-muted-foreground">
          {card.runtimeLabel} · {card.modelLabel}
        </span>
      </span>
    </>
  );

  return (
    <article
      className="flex min-w-0 flex-col gap-3 rounded-2xl border border-border/70 bg-background/80 p-4 shadow-xs"
      data-testid={`persona-agent-row-${card.id}`}
    >
      <div className="flex min-w-0 items-start gap-3">
        {persona && onOpenPersonaProfile ? (
          <button
            aria-label={`${card.displayName} template profile`}
            className="flex min-w-0 flex-1 items-center gap-3 rounded-lg text-left outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
            onClick={() => onOpenPersonaProfile(persona)}
            type="button"
          >
            {identity}
          </button>
        ) : (
          <div className="flex min-w-0 flex-1 items-center gap-3 rounded-lg text-left">
            {identity}
          </div>
        )}
        {mutationActions ? (
          <PersonaActionsMenu
            isActionPending={isActionPending}
            isPending={isPending}
            persona={mutationActions.persona}
            onDeactivate={mutationActions.onDeactivatePersona}
            onDelete={mutationActions.onDeletePersona}
            onDuplicate={mutationActions.onDuplicatePersona}
            onEdit={mutationActions.onEditPersona}
            onShare={mutationActions.onSharePersona}
            onSnapshot={mutationActions.onSnapshotPersona}
          />
        ) : null}
      </div>

      <div className="flex min-w-0 items-end justify-between gap-3 border-t border-border/60 pt-3">
        <div className="min-w-0 flex-1">
          <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
            {deployments.length > 0 ? "Deployed as →" : "Not deployed"}
          </p>
          {deployments.length > 0 ? (
            <div className="mt-1.5 flex flex-wrap gap-1.5">
              {deployments.map((deployment) => (
                <button
                  aria-label={`Open ${deployment.name} agent profile`}
                  className="max-w-full truncate rounded-full border border-border/70 bg-muted/60 px-2.5 py-1 text-xs font-medium text-foreground transition-colors hover:bg-muted focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                  data-testid={`persona-deployment-${card.id}-${deployment.pubkey}`}
                  key={deployment.pubkey}
                  onClick={() => onOpenAgentProfile(deployment.pubkey)}
                  type="button"
                >
                  {deployment.name}
                </button>
              ))}
            </div>
          ) : (
            <p className="mt-1 text-xs text-muted-foreground">
              {card.readOnly
                ? "No deployed instances reported."
                : "Deploy to create an instance."}
            </p>
          )}
        </div>
        {mutationActions ? (
          <Button
            aria-label={`Deploy ${mutationActions.persona.displayName}`}
            disabled={isPending || isStarting}
            onClick={() =>
              mutationActions.onDeployPersona(mutationActions.persona)
            }
            size="sm"
            type="button"
            variant="outline"
          >
            <Rocket className="h-4 w-4" />
            {isStarting ? "Deploying…" : "Deploy"}
          </Button>
        ) : null}
      </div>
    </article>
  );
}

function LibraryLoadingSkeleton() {
  return (
    <div className="grid grid-cols-1 gap-3 [@container(min-width:38rem)]:grid-cols-2">
      {[0, 1].map((item) => (
        <div
          className="flex items-center gap-3 rounded-2xl border border-border/70 bg-background/80 p-4"
          key={item}
        >
          <Skeleton className="h-12 w-12 shrink-0 rounded-full" />
          <div className="min-w-0 flex-1 space-y-2">
            <Skeleton className="h-4 w-28" />
            <Skeleton className="h-3 w-40" />
          </div>
        </div>
      ))}
    </div>
  );
}
