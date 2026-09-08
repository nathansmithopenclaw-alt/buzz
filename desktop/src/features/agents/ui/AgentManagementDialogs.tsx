import { useAgentManagement } from "@/features/agents/useAgentManagement";
import { useAgentTeamManagement } from "@/features/agents/useAgentTeamManagement";
import { ProjectChannelRequestDialog } from "@/features/projects/ui/ProjectChannelRequestDialog";
import { AgentCardDialogs } from "./AgentCardViewerDialog";
import { AgentDialog } from "./AgentDialog";
import { AgentTeamManagementReviewDialog } from "./AgentTeamManagementReviewDialog";
import { AgentGlobalConfigReviewDialog } from "./AgentGlobalConfigReviewDialog";

/** Global review surfaces opened by owned agents through the Buzz harness. */
export function AgentManagementDialogs() {
  const management = useAgentManagement();
  const teamManagement = useAgentTeamManagement();
  const requestId = management.request?.requestId;

  return (
    <>
      {management.request?.action === "create" ? (
        <AgentDialog
          definitionError={
            management.error ? new Error(management.error) : null
          }
          initialValues={management.createInitialValues}
          isDefinitionPending={management.isPending}
          key={requestId}
          mode="definition"
          onOpenChange={(open) => {
            if (!open && requestId) management.dismiss(requestId);
          }}
          onSubmitDefinition={management.submitCreate}
          runtimes={management.runtimes}
          runtimeCatalogStatus={management.runtimeCatalogStatus}
        />
      ) : null}
      {management.request?.action === "update" ? (
        <AgentDialog
          description=""
          error={
            management.error || management.editError
              ? new Error(management.error ?? management.editError ?? "")
              : null
          }
          initialValues={management.editInitialValues}
          isPending={management.isPending}
          key={requestId}
          mode="definition-edit"
          onOpenChange={(open) => {
            if (!open && requestId) management.dismiss(requestId);
          }}
          onSubmit={management.submitUpdate}
          open
          runtimes={management.runtimes}
          runtimeCatalogStatus={management.runtimeCatalogStatus}
          submitLabel="Save changes"
          title="Edit agent"
        />
      ) : null}
      <AgentGlobalConfigReviewDialog
        error={management.error}
        isPending={management.globalConfigPending}
        onApprove={() => {
          void management.approveGlobalConfig();
        }}
        onDismiss={() => {
          if (requestId) management.dismiss(requestId);
        }}
        request={
          management.request?.action === "global_config"
            ? management.request
            : null
        }
      />
      <ProjectChannelRequestDialog />
      <AgentCardDialogs />
      <AgentTeamManagementReviewDialog
        error={teamManagement.reviewError}
        isPending={teamManagement.isReviewPending}
        onApprove={() => {
          void teamManagement.approvePendingReview();
        }}
        onDismiss={() => {
          void teamManagement.dismissPendingReview();
        }}
        request={teamManagement.pendingReview?.request ?? null}
      />
    </>
  );
}
