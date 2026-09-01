import {
  globalConfigAuditTarget,
  globalConfigPatchFromRequest,
  type AgentManagementRequest,
} from "./agentManagement";
import { getDefaultPersonaRuntime } from "./lib/resolvePersonaRuntime";
import {
  completeAgentManagementRequest,
  publishAgentManagementAuditReceipt,
  publishAgentManagementResult,
} from "@/shared/api/tauriManagedAgents";
import {
  applyDelegatedGlobalAgentConfig,
  getGlobalAgentConfig,
} from "@/shared/api/tauriGlobalAgentConfig";
import { discoverAcpRuntimes } from "@/shared/api/tauriAcpDiscovery";

type GlobalRequest = Extract<
  AgentManagementRequest,
  { action: "global_config" }
>;
type SaveResult = Awaited<ReturnType<typeof applyDelegatedGlobalAgentConfig>>;

type ApplyBridge = {
  getConfig: typeof getGlobalAgentConfig;
  discoverRuntimes: typeof discoverAcpRuntimes;
  applyPatch: typeof applyDelegatedGlobalAgentConfig;
};

export async function applyDelegatedGlobalConfigWith(
  request: GlobalRequest,
  bridge: ApplyBridge,
): Promise<SaveResult> {
  const currentConfig = await bridge.getConfig();
  const runtimeId =
    request.request.preferredRuntime ?? currentConfig.preferred_runtime;
  const runtimes = request.request.effort
    ? await bridge.discoverRuntimes()
    : [];
  const effortRuntime = request.request.effort
    ? runtimeId
      ? runtimes.find((runtime) => runtime.id === runtimeId)
      : getDefaultPersonaRuntime(runtimes)
    : null;
  return bridge.applyPatch(
    globalConfigPatchFromRequest(request, effortRuntime?.thinkingEnvVar),
  );
}

export function applyDelegatedGlobalConfig(
  request: GlobalRequest,
): Promise<SaveResult> {
  return applyDelegatedGlobalConfigWith(request, {
    getConfig: getGlobalAgentConfig,
    discoverRuntimes: discoverAcpRuntimes,
    applyPatch: applyDelegatedGlobalAgentConfig,
  });
}

export async function completeDelegatedGlobalConfigSuccess(input: {
  requestId: string;
  sourcePubkey: string;
  requesterName: string;
  request: GlobalRequest;
  saveResult: SaveResult;
}): Promise<void> {
  const result = {
    saved: true,
    changedFields: Object.keys(input.request.request).filter(
      (field) => field !== "channelId",
    ),
    restartedCount: input.saveResult.restarted_count,
    failedRestartCount: input.saveResult.failed_restart_count,
  };
  await completeAgentManagementRequest(input.requestId, true, result);
  await publishAgentManagementAuditReceipt({
    requestId: input.requestId,
    sourcePubkey: input.sourcePubkey,
    requesterName: input.requesterName,
    capability: "global_config",
    target: globalConfigAuditTarget(result),
    succeeded: true,
  });
  await publishAgentManagementResult({
    requestId: input.requestId,
    sourcePubkey: input.sourcePubkey,
    result,
  });
}

export async function completeDelegatedGlobalConfigFailure(input: {
  requestId: string;
  sourcePubkey: string;
  requesterName: string;
  cause: unknown;
}): Promise<void> {
  const message =
    input.cause instanceof Error
      ? input.cause.message
      : "Global config update failed.";
  const result = { error: message };
  await completeAgentManagementRequest(input.requestId, false, result);
  await publishAgentManagementAuditReceipt({
    requestId: input.requestId,
    sourcePubkey: input.sourcePubkey,
    requesterName: input.requesterName,
    capability: "global_config",
    target: `global agent config update: ${message}`,
    succeeded: false,
  });
  await publishAgentManagementResult({
    requestId: input.requestId,
    sourcePubkey: input.sourcePubkey,
    result,
  });
}
