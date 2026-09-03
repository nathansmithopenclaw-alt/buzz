import { Rocket } from "lucide-react";

import { resolveAgentCardModelLabel } from "@/features/agents/lib/agentCardModelLabel";
import { effectiveAgentDescription } from "@/features/agents/lib/agentDescription";
import type { FleetProjectionPersona } from "@/features/agents/fleetProjection/types";
import type { AgentPersona } from "@/shared/api/types";
import { Badge } from "@/shared/ui/badge";
import { AgentIdentityCard } from "./AgentIdentityCard";
import { PersonaActionsMenu } from "./PersonaActionsMenu";

type HostUndeployedPersonaCardProps = {
  defaultModel: string;
  isActionPending: boolean;
  isPending: boolean;
  isStarting: boolean;
  persona: AgentPersona;
  onDeactivatePersona: (persona: AgentPersona) => void;
  onDeletePersona: (persona: AgentPersona) => void;
  onDeployPersona: (persona: AgentPersona) => void;
  onDuplicatePersona: (persona: AgentPersona) => void;
  onEditPersona: (persona: AgentPersona) => void;
  onOpenPersonaProfile: (persona: AgentPersona) => void;
  onSharePersona: (persona: AgentPersona) => void;
  onSnapshotPersona: (persona: AgentPersona) => void;
};

type ObserverUndeployedPersonaCardProps = {
  persona: FleetProjectionPersona;
};

export function HostUndeployedPersonaCard({
  defaultModel,
  isActionPending,
  isPending,
  isStarting,
  persona,
  onDeactivatePersona,
  onDeletePersona,
  onDeployPersona,
  onDuplicatePersona,
  onEditPersona,
  onOpenPersonaProfile,
  onSharePersona,
  onSnapshotPersona,
}: HostUndeployedPersonaCardProps) {
  const modelLabel = resolveAgentCardModelLabel({
    agent: undefined,
    personaModel: persona.model,
    defaultModel,
  });
  const runtimeLabel = persona.runtime?.trim() || "Default runtime";

  return (
    <AgentIdentityCard
      actions={
        <div className="flex items-center gap-1">
          <button
            aria-label={`Deploy ${persona.displayName}`}
            className="relative z-40 flex h-7 items-center gap-1 rounded-md border border-border/70 bg-background/90 px-2 text-xs font-medium text-foreground hover:bg-muted"
            disabled={isPending || isStarting}
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              onDeployPersona(persona);
            }}
            type="button"
          >
            <Rocket className="h-3.5 w-3.5" />
            {isStarting ? "Deploying…" : "Deploy"}
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
      }
      ariaLabel={`${persona.displayName} template profile`}
      avatarUrl={persona.avatarUrl}
      dataTestId={`persona-agent-row-${persona.id}`}
      label={persona.displayName}
      modelLabel={`${runtimeLabel} · ${modelLabel}`}
      subtitle={effectiveAgentDescription(persona)}
      statusBadge={<Badge variant="secondary">Not deployed</Badge>}
      onClick={() => onOpenPersonaProfile(persona)}
    />
  );
}

export function ObserverUndeployedPersonaCard({
  persona,
}: ObserverUndeployedPersonaCardProps) {
  return (
    <AgentIdentityCard
      ariaLabel={`${persona.displayName} definition`}
      avatarUrl={persona.avatarUrl}
      dataTestId={`persona-agent-row-${persona.personaId}`}
      label={persona.displayName}
      modelLabel={`${persona.runtime?.trim() || "Default runtime"} · ${persona.model?.trim() || "Default model"}`}
      subtitle={persona.description}
      statusBadge={<Badge variant="secondary">Not deployed</Badge>}
    />
  );
}
