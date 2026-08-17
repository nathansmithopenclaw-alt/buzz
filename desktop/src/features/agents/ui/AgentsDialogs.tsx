import * as React from "react";

import {
  consumePendingSnapshotImport,
  subscribeSnapshotImport,
} from "@/features/agents/openSnapshotImportFromUrlEvent";
import type { AgentPersona } from "@/shared/api/types";
import { AddAgentToChannelDialog } from "./AddAgentToChannelDialog";
import { AddTeamToChannelDialog } from "./AddTeamToChannelDialog";
import { AgentDefaultsDialog } from "./AgentDefaultsDialog";
import { AgentDialog } from "./AgentDialog";
import { AgentSnapshotExportDialog } from "./AgentSnapshotExportDialog";
import { AgentSnapshotImportDialog } from "./AgentSnapshotImportDialog";
import { buildAgentMemorySources } from "./agentDeployments";
import { PersonaCatalogDialog } from "./PersonaCatalogDialog";
import { PersonaDeleteDialog } from "./PersonaDeleteDialog";
import { PersonaShareDialog } from "./PersonaShareDialog";
import { TeamDeleteDialog } from "./TeamDeleteDialog";
import { TeamDialog } from "./TeamDialog";
import { TeamShareDialog } from "./TeamShareDialog";
import { TeamSnapshotExportDialog } from "./TeamSnapshotExportDialog";
import { TeamSnapshotImportDialog } from "./TeamSnapshotImportDialog";
import type { useManagedAgentActions } from "./useManagedAgentActions";
import type { usePersonaActions } from "./usePersonaActions";
import type { useTeamActions } from "./useTeamActions";

type AgentsDialogsProps = {
  agents: ReturnType<typeof useManagedAgentActions>;
  aiDefaultsOpen: boolean;
  aiDefaultsReturnFocusRef: React.RefObject<HTMLButtonElement | null>;
  personas: ReturnType<typeof usePersonaActions>;
  snapshotImportInputRef: React.RefObject<HTMLInputElement | null>;
  teamActions: ReturnType<typeof useTeamActions>;
  onAiDefaultsOpenChange: (open: boolean) => void;
  onOpenPersonaProfile: (persona: AgentPersona) => void;
};

export function AgentsDialogs({
  agents,
  aiDefaultsOpen,
  aiDefaultsReturnFocusRef,
  personas,
  snapshotImportInputRef,
  teamActions,
  onAiDefaultsOpenChange,
  onOpenPersonaProfile,
}: AgentsDialogsProps) {
  const personaShareDeployments = personas.personaToShare?.deployments;
  const personaShareMemorySources = React.useMemo(
    () => buildAgentMemorySources(personaShareDeployments ?? []),
    [personaShareDeployments],
  );
  const personaExportDeployments =
    personas.personaToExportSnapshot?.deployments;
  const personaExportMemorySources = React.useMemo(
    () => buildAgentMemorySources(personaExportDeployments ?? []),
    [personaExportDeployments],
  );

  // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only; import handlers are stable action-controller members
  React.useEffect(() => {
    const importSnapshot = (
      fileBytes: number[],
      fileName: string,
      snapshotKind: "agent" | "team",
    ) => {
      if (snapshotKind === "team") {
        void teamActions.handleImportTeamSnapshotFile(fileBytes, fileName);
        return;
      }
      void personas.handleImportSnapshotFile(fileBytes, fileName);
    };

    const pending = consumePendingSnapshotImport();
    if (pending) {
      importSnapshot(pending.fileBytes, pending.fileName, pending.snapshotKind);
    }

    return subscribeSnapshotImport(({ fileBytes, fileName, snapshotKind }) =>
      importSnapshot(fileBytes, fileName, snapshotKind),
    );
  }, []);

  return (
    <>
      <AgentDefaultsDialog
        onOpenChange={onAiDefaultsOpenChange}
        open={aiDefaultsOpen}
        returnFocusRef={aiDefaultsReturnFocusRef}
      />

      {agents.agentToAddToChannel ? (
        <AddAgentToChannelDialog
          agent={agents.agentToAddToChannel}
          onAdded={agents.handleAddedToChannel}
          onOpenChange={(open) => {
            if (!open) agents.setAgentToAddToChannel(null);
          }}
          open
        />
      ) : null}

      {personas.personaDialogState ? (
        <AgentDialog
          description={personas.personaDialogState.description}
          error={
            personas.updatePersonaMutation.error instanceof Error
              ? personas.updatePersonaMutation.error
              : personas.updatePersonaAndPublishMutation.error instanceof Error
                ? personas.updatePersonaAndPublishMutation.error
                : personas.createPersonaMutation.error instanceof Error
                  ? personas.createPersonaMutation.error
                  : null
          }
          initialValues={personas.personaDialogState.initialValues}
          isPending={personas.isPending}
          mode="definition-edit"
          onOpenChange={(open) => {
            if (!open) personas.setPersonaDialogState(null);
          }}
          onSubmit={(input, options) =>
            personas.handleSubmit(
              input,
              undefined,
              undefined,
              undefined,
              options,
            )
          }
          open
          publishCatalogUpdatesOnSave={
            "id" in personas.personaDialogState.initialValues &&
            personas.sharedCatalogPersonaIdSet.has(
              personas.personaDialogState.initialValues.id,
            )
          }
          runtimes={personas.acpRuntimesQuery.data ?? []}
          runtimeCatalogStatus={
            personas.acpRuntimesQuery.isLoading
              ? "loading"
              : personas.acpRuntimesQuery.isError
                ? "error"
                : "ready"
          }
          submitLabel={personas.personaDialogState.submitLabel}
          title={personas.personaDialogState.title}
        />
      ) : null}

      {personas.personaToDelete ? (
        <PersonaDeleteDialog
          instanceCount={
            agents.managedAgents.filter(
              (agent) => agent.personaId === personas.personaToDelete?.id,
            ).length
          }
          onConfirm={(persona) => {
            void personas.handleDelete(persona);
          }}
          onOpenChange={(open) => {
            if (!open) personas.setPersonaToDelete(null);
          }}
          open
          persona={personas.personaToDelete}
        />
      ) : null}

      {personas.personaToShare ? (
        <PersonaShareDialog
          catalogShareLevel={personas.getPersonaCatalogShareLevel(
            personas.personaToShare.persona,
          )}
          effectiveAvatarUrl={personas.personaToShare.effectiveAvatarUrl}
          isPending={personas.isPending}
          memorySources={personaShareMemorySources}
          onCatalogShareLevelChange={(shareLevel) => {
            const target = personas.personaToShare;
            if (!target) return;
            void personas.setPersonaCatalogShareLevel(
              target.persona,
              shareLevel,
            );
          }}
          onExport={(memorySourcePubkey) => {
            const target = personas.personaToShare;
            if (!target) return;
            personas.setPersonaToShare(null);
            personas.setPersonaToExportSnapshot({
              ...target,
              preferredMemorySourcePubkey: memorySourcePubkey,
            });
          }}
          onOpenChange={(open) => {
            if (!open) personas.setPersonaToShare(null);
          }}
          open
          persona={personas.personaToShare.persona}
        />
      ) : null}

      {personas.personaToExportSnapshot ? (
        <AgentSnapshotExportDialog
          agentName={personas.personaToExportSnapshot.persona.displayName}
          effectiveAvatarUrl={
            personas.personaToExportSnapshot.effectiveAvatarUrl
          }
          isSavePending={personas.isPending}
          memorySources={personaExportMemorySources}
          onOpenChange={(open) => {
            if (!open) personas.setPersonaToExportSnapshot(null);
          }}
          onSaveFile={(
            memoryLevel,
            format,
            memorySourcePubkey,
            snapshotAvatarUrl,
          ) => {
            const target = personas.personaToExportSnapshot;
            if (!target) return;
            personas.handleExportSnapshot(
              target.persona,
              memorySourcePubkey,
              snapshotAvatarUrl,
              memoryLevel,
              format,
            );
          }}
          open
          preferredMemorySourcePubkey={
            personas.personaToExportSnapshot.preferredMemorySourcePubkey
          }
        />
      ) : null}

      {personas.snapshotImportState ? (
        <AgentSnapshotImportDialog
          confirmError={personas.snapshotImportConfirmError}
          isConfirming={personas.isSnapshotImportConfirming}
          onConfirm={(keepAllowlist) => {
            void personas.handleConfirmSnapshotImport(keepAllowlist);
          }}
          onOpenChange={(open) => {
            if (!open) personas.closeSnapshotImportDialog();
          }}
          open
          preview={personas.snapshotImportState.preview}
          result={personas.snapshotImportResult}
        />
      ) : null}

      {personas.isCatalogDialogOpen ? (
        <PersonaCatalogDialog
          createContent={({ onDirtyChange, onRequestClose }) => (
            <AgentDialog
              definitionError={
                personas.createPersonaMutation.error instanceof Error
                  ? personas.createPersonaMutation.error
                  : null
              }
              embedded
              isDefinitionPending={personas.isPending}
              mode="definition"
              onDirtyChange={onDirtyChange}
              onOpenChange={(open) => {
                if (!open) onRequestClose();
              }}
              onSubmitDefinition={personas.handleSubmit}
              runtimes={personas.acpRuntimesQuery.data ?? []}
              runtimeCatalogStatus={
                personas.acpRuntimesQuery.isLoading
                  ? "loading"
                  : personas.acpRuntimesQuery.isError
                    ? "error"
                    : "ready"
              }
              submitLabel="Add agent"
            />
          )}
          error={
            personas.catalogQuery.error instanceof Error
              ? personas.catalogQuery.error
              : null
          }
          feedbackErrorMessage={
            personas.personaFeedbackSurface === "catalog"
              ? personas.personaErrorMessage
              : null
          }
          feedbackNoticeMessage={
            personas.personaFeedbackSurface === "catalog"
              ? personas.personaNoticeMessage
              : null
          }
          isLoading={personas.catalogQuery.isLoading}
          isPending={personas.isPending}
          onClearFeedback={() => personas.clearFeedback("catalog")}
          onImportFile={(fileBytes, fileName) => {
            void personas.handleImportSnapshotFile(fileBytes, fileName);
          }}
          onOpenChange={personas.setIsCatalogDialogOpen}
          onSelectPersona={async (persona, active) => {
            const addedPersona = await personas.handleSetActive(
              persona,
              active,
              "catalog",
            );
            if (!active || !addedPersona) return;
            personas.setIsCatalogDialogOpen(false);
            onOpenPersonaProfile(addedPersona);
          }}
          open
          personas={personas.catalogPersonas}
        />
      ) : null}

      {teamActions.teamDialogState ? (
        <TeamDialog
          description={teamActions.teamDialogState.description}
          error={
            teamActions.updateTeamMutation.error instanceof Error
              ? teamActions.updateTeamMutation.error
              : teamActions.createTeamMutation.error instanceof Error
                ? teamActions.createTeamMutation.error
                : null
          }
          initialValues={teamActions.teamDialogState.initialValues}
          isPending={
            teamActions.createTeamMutation.isPending ||
            teamActions.updateTeamMutation.isPending
          }
          onDeleteRemovedPersonas={teamActions.handleDeleteRemovedPersonas}
          onOpenChange={(open) => {
            if (!open) teamActions.setTeamDialogState(null);
          }}
          onSubmit={teamActions.handleTeamSubmit}
          open
          personas={personas.libraryPersonas}
          submitLabel={teamActions.teamDialogState.submitLabel}
          title={teamActions.teamDialogState.title}
        />
      ) : null}

      {teamActions.teamToDelete ? (
        <TeamDeleteDialog
          onConfirm={(team) => {
            void teamActions.handleDeleteTeam(team);
          }}
          onOpenChange={(open) => {
            if (!open) teamActions.setTeamToDelete(null);
          }}
          open
          team={teamActions.teamToDelete}
        />
      ) : null}

      {teamActions.teamToAddToChannel ? (
        <AddTeamToChannelDialog
          onDeployed={teamActions.handleTeamDeployed}
          onOpenChange={(open) => {
            if (!open) teamActions.setTeamToAddToChannel(null);
          }}
          open
          personas={personas.libraryPersonas}
          team={teamActions.teamToAddToChannel}
        />
      ) : null}

      {teamActions.teamToShare ? (
        <TeamShareDialog
          isPending={
            teamActions.createTeamMutation.isPending ||
            teamActions.updateTeamMutation.isPending ||
            teamActions.deleteTeamMutation.isPending
          }
          onExport={() => {
            const team = teamActions.teamToShare;
            if (!team) return;
            teamActions.setTeamToShare(null);
            teamActions.openExportSnapshot(team);
          }}
          onOpenChange={(open) => {
            if (!open) teamActions.setTeamToShare(null);
          }}
          open
          team={teamActions.teamToShare}
        />
      ) : null}

      {teamActions.teamToExport ? (
        <TeamSnapshotExportDialog
          isSavePending={teamActions.exportTeamSnapshotMutation.isPending}
          onOpenChange={(open) => {
            if (!open) teamActions.setTeamToExport(null);
          }}
          onSaveFile={(memoryLevel, format) => {
            const team = teamActions.teamToExport;
            if (!team) return;
            teamActions.handleExportTeamSnapshot(team, memoryLevel, format);
          }}
          open
          team={teamActions.teamToExport}
        />
      ) : null}

      {teamActions.teamSnapshotImportState ? (
        <TeamSnapshotImportDialog
          confirmError={teamActions.teamSnapshotImportConfirmError}
          isConfirming={teamActions.isTeamSnapshotImportConfirming}
          onConfirm={(keepAllowlist) => {
            void teamActions.handleConfirmTeamSnapshotImport(keepAllowlist);
          }}
          onOpenChange={(open) => {
            if (!open) teamActions.closeTeamSnapshotImportDialog();
          }}
          open
          preview={teamActions.teamSnapshotImportState.preview}
          result={teamActions.teamSnapshotImportResult}
        />
      ) : null}

      <input
        accept=".agent.json,.agent.png,.team.json,.team.png"
        className="hidden"
        data-testid="snapshot-import-input"
        onChange={(event) => {
          const file = event.target.files?.[0];
          if (!file) return;
          const reader = new FileReader();
          reader.onload = () => {
            const fileBytes = Array.from(
              new Uint8Array(reader.result as ArrayBuffer),
            );
            const normalizedName = file.name.toLowerCase();
            if (
              normalizedName.endsWith(".team.json") ||
              normalizedName.endsWith(".team.png")
            ) {
              void teamActions.handleImportTeamSnapshotFile(
                fileBytes,
                file.name,
              );
            } else {
              void personas.handleImportSnapshotFile(fileBytes, file.name);
            }
          };
          reader.readAsArrayBuffer(file);
          event.target.value = "";
        }}
        ref={snapshotImportInputRef}
        type="file"
      />
    </>
  );
}
