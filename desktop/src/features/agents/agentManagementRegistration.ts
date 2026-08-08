import type { AgentManagementRequest } from "./agentManagement";
import {
  publishAgentManagementApplyingResult,
  publishAgentManagementDraftResult,
  registerAgentManagementRequest,
  type RegisterManagementRequestResult,
} from "@/shared/api/tauriManagedAgents";

type RegistrationBridge = {
  register: typeof registerAgentManagementRequest;
  publishApplying: typeof publishAgentManagementApplyingResult;
  publishDraft: typeof publishAgentManagementDraftResult;
};

const registrationBridge: RegistrationBridge = {
  register: registerAgentManagementRequest,
  publishApplying: publishAgentManagementApplyingResult,
  publishDraft: publishAgentManagementDraftResult,
};

export type AgentManagementRegistrationRoute =
  | { kind: "delegated_apply" }
  | { kind: "draft" }
  | { kind: "duplicate_applying" }
  | {
      kind: "terminal_duplicate";
      registered: RegisterManagementRequestResult;
    };

/**
 * Owns the register-to-result seam for an incoming management request.
 *
 * An authorized apply is already durable in the `applying` state, so it must
 * proceed directly to execution. Only draft dispositions may publish the
 * queued-for-review result.
 */
export async function registerAndRouteAgentManagementRequest(
  agentPubkey: string,
  request: AgentManagementRequest,
  bridge: RegistrationBridge = registrationBridge,
): Promise<AgentManagementRegistrationRoute> {
  const updateChanges = request.action === "update" ? request.request : null;
  const capability =
    request.action === "create"
      ? "agent_creation"
      : updateChanges?.envVars &&
          Object.keys(updateChanges).every((key) =>
            ["channelId", "agentName", "envVars"].includes(key),
          )
        ? "env_vars"
        : "agent_config";
  const registered = await bridge.register({
    sourcePubkey: agentPubkey,
    requestId: request.requestId,
    capability,
    intent: request.intent,
    payload: request,
  });

  switch (registered.disposition) {
    case "apply":
      return { kind: "delegated_apply" };
    case "draft":
    case "duplicate_draft":
      if (request.intent === "apply") {
        await bridge.publishDraft({
          requestId: request.requestId,
          sourcePubkey: agentPubkey,
        });
      }
      return { kind: "draft" };
    case "duplicate_applying":
      await bridge.publishApplying({
        requestId: request.requestId,
        sourcePubkey: agentPubkey,
      });
      return { kind: "duplicate_applying" };
    default:
      return { kind: "terminal_duplicate", registered };
  }
}
