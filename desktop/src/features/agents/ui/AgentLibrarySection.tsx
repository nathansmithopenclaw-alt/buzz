import { Library, Rocket } from "lucide-react";

import { resolveAgentCardModelLabel } from "@/features/agents/lib/agentCardModelLabel";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { AgentPersona } from "@/shared/api/types";
import type { ProfilePanelOpenOptions } from "@/shared/context/ProfilePanelContext";
import { Button } from "@/shared/ui/button";
import { SectionHeader } from "@/shared/ui/PageHeader";
import { Skeleton } from "@/shared/ui/skeleton";
import type { AgentDeployment } from "./agentDeployments";
import { IdentityInitialsAvatar } from "./IdentityInitialsAvatar";
import { PersonaActionsMenu } from "./PersonaActionsMenu";

type AgentLibrarySectionProps = {
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

export function AgentLibrarySection({
  defaultModel,
  deploymentsByPersonaId,
  error,
  isActionPending,
  isLoading,
  isPending,
  personas,
  startingPersonaIds,
  onBrowseCatalog,
  onDeactivatePersona,
  onDeletePersona,
  onDeployPersona,
  onDuplicatePersona,
  onEditPersona,
  onOpenAgentProfile,
  onOpenPersonaProfile,
  onSharePersona,
  onSnapshotPersona,
}: AgentLibrarySectionProps) {
  return (
    <section
      className="relative space-y-4"
      data-testid="agents-library-personas"
    >
      <SectionHeader
        action={
          <Button
            disabled={isPending}
            onClick={onBrowseCatalog}
            size="sm"
            type="button"
            variant="outline"
          >
            <Library className="h-4 w-4" />
            Browse catalog
          </Button>
        }
        description="Reusable templates for deploying agents into your fleet."
        title="Library"
      />

      {isLoading ? <LibraryLoadingSkeleton /> : null}

      {!isLoading && !error && personas.length === 0 ? (
        <div className="rounded-2xl border border-dashed border-border/80 px-5 py-6">
          <p className="font-medium">No agent templates yet</p>
          <p className="mt-1 text-sm text-muted-foreground">
            Create a template or add one from the catalog.
          </p>
          <Button
            className="mt-4"
            disabled={isPending}
            onClick={onBrowseCatalog}
            size="sm"
            type="button"
          >
            Browse catalog
          </Button>
        </div>
      ) : null}

      {!isLoading && personas.length > 0 ? (
        <div
          className="grid grid-cols-1 gap-3 [@container(min-width:38rem)]:grid-cols-2"
          data-testid="agent-library-grid"
        >
          {personas.map((persona) => {
            const deployments = deploymentsByPersonaId.get(persona.id) ?? [];
            return (
              <LibraryTemplateCard
                defaultModel={defaultModel}
                deployments={deployments}
                isActionPending={isActionPending}
                isPending={isPending}
                isStarting={startingPersonaIds.has(persona.id)}
                key={persona.id}
                persona={persona}
                onDeactivatePersona={onDeactivatePersona}
                onDeletePersona={onDeletePersona}
                onDeployPersona={onDeployPersona}
                onDuplicatePersona={onDuplicatePersona}
                onEditPersona={onEditPersona}
                onOpenAgentProfile={onOpenAgentProfile}
                onOpenPersonaProfile={onOpenPersonaProfile}
                onSharePersona={() => onSharePersona(persona, deployments)}
                onSnapshotPersona={() =>
                  onSnapshotPersona(persona, deployments)
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
  defaultModel,
  deployments,
  isActionPending,
  isPending,
  isStarting,
  persona,
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
  defaultModel: string;
  deployments: AgentDeployment[];
  isActionPending: boolean;
  isPending: boolean;
  isStarting: boolean;
  persona: AgentPersona;
  onDeactivatePersona: (persona: AgentPersona) => void;
  onDeletePersona: (persona: AgentPersona) => void;
  onDeployPersona: (persona: AgentPersona) => void;
  onDuplicatePersona: (persona: AgentPersona) => void;
  onEditPersona: (persona: AgentPersona) => void;
  onOpenAgentProfile: (pubkey: string) => void;
  onOpenPersonaProfile: (persona: AgentPersona) => void;
  onSharePersona: () => void;
  onSnapshotPersona: () => void;
}) {
  const runtimeLabel = persona.runtime?.trim() || "Default runtime";
  const modelLabel = resolveAgentCardModelLabel({
    agent: undefined,
    personaModel: persona.model,
    defaultModel,
  });

  return (
    <article
      className="flex min-w-0 flex-col gap-3 rounded-2xl border border-border/70 bg-background/80 p-4 shadow-xs"
      data-testid={`persona-agent-row-${persona.id}`}
    >
      <div className="flex min-w-0 items-start gap-3">
        <button
          aria-label={`${persona.displayName} template profile`}
          className="flex min-w-0 flex-1 items-center gap-3 rounded-lg text-left outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
          onClick={() => onOpenPersonaProfile(persona)}
          type="button"
        >
          {persona.avatarUrl?.trim() ? (
            <ProfileAvatar
              avatarUrl={persona.avatarUrl}
              className="h-12 w-12 shrink-0 border-2 border-background bg-muted shadow-none"
              iconClassName="h-5 w-5"
              label={persona.displayName}
            />
          ) : (
            <IdentityInitialsAvatar
              className="h-12 w-12 shrink-0 border-0 shadow-none"
              label={persona.displayName}
              size={48}
            />
          )}
          <span className="min-w-0 flex-1">
            <span className="block truncate text-sm font-semibold">
              {persona.displayName}
            </span>
            <span className="mt-0.5 block truncate text-xs text-muted-foreground">
              {runtimeLabel} · {modelLabel}
            </span>
          </span>
        </button>
        <PersonaActionsMenu
          isActionPending={isActionPending}
          isPending={isPending}
          persona={persona}
          onDeactivate={onDeactivatePersona}
          onDelete={onDeletePersona}
          onDuplicate={onDuplicatePersona}
          onEdit={onEditPersona}
          onShare={onSharePersona}
          onSnapshot={onSnapshotPersona}
        />
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
                  data-testid={`persona-deployment-${persona.id}-${deployment.pubkey}`}
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
              Deploy to create an instance.
            </p>
          )}
        </div>
        <Button
          aria-label={`Deploy ${persona.displayName}`}
          disabled={isPending || isStarting}
          onClick={() => onDeployPersona(persona)}
          size="sm"
          type="button"
          variant="outline"
        >
          <Rocket className="h-4 w-4" />
          {isStarting ? "Deploying…" : "Deploy"}
        </Button>
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
