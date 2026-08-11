import { useAgentManagement } from "@/features/agents/useAgentManagement";
import { useAgentTeamManagement } from "@/features/agents/useAgentTeamManagement";
import { AgentCardDialogs } from "./AgentCardViewerDialog";
import { AgentDialog } from "./AgentDialog";
import { AgentTeamManagementReviewDialog } from "./AgentTeamManagementReviewDialog";
import { AgentGlobalConfigReviewDialog } from "./AgentGlobalConfigReviewDialog";

/** Global review surfaces opened by owned agents through the Buzz harness. */
export function AgentManagementDialogs() {
  const management = useAgentManagement();
  const teamManagement = useAgentTeamManagement();

  return (
    <>
      {management.request?.action === "create" ? (
        <AgentDialog
          definitionError={
            management.error ? new Error(management.error) : null
          }
          initialValues={management.createInitialValues}
          isDefinitionPending={management.isPending}
          mode="definition"
          onOpenChange={(open) => {
            if (!open) management.dismiss();
          }}
          onSubmitDefinition={management.submitCreate}
          runtimes={management.runtimes}
          runtimeCatalogStatus={management.runtimeCatalogStatus}
        />
      ) : null}
      {management.request?.action === "update" ? (
        <AgentDialog
          description=""
          error={management.editError ? new Error(management.editError) : null}
          initialValues={management.editInitialValues}
          isPending={management.isPending}
          mode="definition-edit"
          onOpenChange={(open) => {
            if (!open) management.dismiss();
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
        onDismiss={management.dismiss}
        request={
          management.request?.action === "global_config"
            ? management.request
            : null
        }
      />
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
