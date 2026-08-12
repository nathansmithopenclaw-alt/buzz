import type { AgentManagementRequest } from "./agentManagement";
import {
  applyDelegatedPersonaEnvVars,
  type DelegatedPersonaEnvVarsResult,
} from "@/shared/api/tauriPersonas";

type EnvRequest = Extract<AgentManagementRequest, { action: "update" }> & {
  request: { envVars: Record<string, string> };
};

type ApplyBridge = {
  applyPatch: typeof applyDelegatedPersonaEnvVars;
};

export type DelegatedEnvActivationCounts = {
  restartedCount: number;
  startedCount: number;
  failedStartCount: number;
};

export function delegatedEnvActivationCounts(
  result: DelegatedPersonaEnvVarsResult,
): DelegatedEnvActivationCounts {
  return {
    restartedCount: result.restarted_count,
    startedCount: result.started_count,
    failedStartCount: result.failed_start_count,
  };
}

export function delegatedEnvAuditTarget(
  agentName: string,
  result: Partial<DelegatedEnvActivationCounts>,
): string {
  const started = result.startedCount ?? 0;
  const restarted = result.restartedCount ?? 0;
  const failed = result.failedStartCount ?? 0;
  if (failed > 0) {
    return `saved env for agent ${agentName}; activation incomplete: started ${started}, restarted ${restarted}, failed ${failed}`;
  }
  if (started + restarted > 0) {
    return `saved env for agent ${agentName}; applied now: started ${started}, restarted ${restarted}`;
  }
  return `saved env for agent ${agentName}; changes take effect on next spawn`;
}

export function applyDelegatedPersonaEnvVarsWith(
  request: EnvRequest,
  personaId: string,
  bridge: ApplyBridge,
): Promise<DelegatedPersonaEnvVarsResult> {
  return bridge.applyPatch({
    id: personaId,
    envVars: request.request.envVars,
  });
}

export function applyDelegatedPersonaEnvVarsRequest(
  request: EnvRequest,
  personaId: string,
): Promise<DelegatedPersonaEnvVarsResult> {
  return applyDelegatedPersonaEnvVarsWith(request, personaId, {
    applyPatch: applyDelegatedPersonaEnvVars,
  });
}
