import type { AgentManagementRequest } from "./agentManagement";
import { capabilityForAgentManagementRequest } from "./agentManagementRegistration";
import {
  delegatedEnvActivationCounts,
  delegatedEnvAuditTarget,
} from "./delegatedPersonaEnvVars";
import {
  completeAgentManagementRequest,
  publishAgentManagementAuditReceipt,
  publishAgentManagementResult,
} from "@/shared/api/tauriManagedAgents";
import type { DelegatedPersonaEnvVarsResult } from "@/shared/api/tauriPersonas";

type UpdateRequest = Extract<AgentManagementRequest, { action: "update" }>;

type CompletionContext = {
  requestId: string;
  sourcePubkey: string;
  requesterName: string;
  request: UpdateRequest;
};

export async function completeDelegatedAgentUpdateSuccess(
  context: CompletionContext,
  activation?: DelegatedPersonaEnvVarsResult,
): Promise<void> {
  const { requestId, sourcePubkey, requesterName, request } = context;
  const changedFields = Object.keys(request.request).filter(
    (field) => field !== "channelId" && field !== "agentName",
  );
  const takeEffect = activation
    ? activation.failed_start_count > 0
      ? "saved_activation_incomplete"
      : activation.restarted_count + activation.started_count > 0
        ? "applied_now"
        : "next_spawn"
    : "next_spawn";
  const activationCounts = activation
    ? delegatedEnvActivationCounts(activation)
    : undefined;
  const result = {
    agentName: request.request.agentName,
    saved: true,
    changedFields,
    instructionAndRuntimeChangesTakeEffect: takeEffect,
    ...(activationCounts ?? {}),
  };
  await completeAgentManagementRequest(requestId, true, result);
  const target = activationCounts
    ? delegatedEnvAuditTarget(request.request.agentName, activationCounts)
    : `saved config for agent ${request.request.agentName}; instruction and runtime changes take effect on next spawn`;
  await publishAgentManagementAuditReceipt({
    requestId,
    sourcePubkey,
    requesterName,
    capability: capabilityForAgentManagementRequest(request),
    target,
    succeeded: true,
  });
  await publishAgentManagementResult({ requestId, sourcePubkey, result });
}

export async function completeDelegatedAgentUpdateFailure(
  context: CompletionContext,
  cause: unknown,
): Promise<void> {
  const { requestId, sourcePubkey, requesterName, request } = context;
  const message =
    cause instanceof Error ? cause.message : "Agent config update failed.";
  const result = { error: message };
  await completeAgentManagementRequest(requestId, false, result);
  const capability = capabilityForAgentManagementRequest(request);
  await publishAgentManagementAuditReceipt({
    requestId,
    sourcePubkey,
    requesterName,
    capability,
    target: `${capability === "env_vars" ? "agent env" : "agent config"} update: ${message}`,
    succeeded: false,
  });
  await publishAgentManagementResult({ requestId, sourcePubkey, result });
}
