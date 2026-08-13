import type { AgentManagementRequest } from "./agentManagement";
import { resurrectStoppedActiveInstances } from "@/shared/api/tauriManagedAgents";
import {
  applyDelegatedPersonaEnvVars,
  type DelegatedPersonaEnvVarsResult,
} from "@/shared/api/tauriPersonas";

type EnvRequest = Extract<AgentManagementRequest, { action: "update" }> & {
  request: { envVars: Record<string, string> };
};

export type ApplyResurrectionResult = {
  started: string[];
  failed: string[];
};

type ApplyBridge = {
  applyPatch: typeof applyDelegatedPersonaEnvVars;
  resurrectStopped: (input: {
    personaId: string;
  }) => Promise<ApplyResurrectionResult>;
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

export function mergeResurrectionIntoActivation(
  activation: DelegatedPersonaEnvVarsResult,
  resurrection: ApplyResurrectionResult,
): DelegatedPersonaEnvVarsResult {
  return {
    ...activation,
    started_count: activation.started_count + resurrection.started.length,
    failed_start_count:
      activation.failed_start_count + resurrection.failed.length,
  };
}

/**
 * Delegated `env_vars` apply path used by `autoApproveUpdate`.
 *
 * The env command only starts agents that flip NotReady→Ready. A killed
 * harness that was already Ready returns 0/0/0 and never respawns unless
 * this wrapper calls `resurrect_stopped_active_instances` afterward.
 * Resurrection failures are logged and counted; the env write stays durable.
 */
export async function applyDelegatedPersonaEnvVarsWith(
  request: EnvRequest,
  personaId: string,
  bridge: ApplyBridge,
): Promise<DelegatedPersonaEnvVarsResult> {
  const activation = await bridge.applyPatch({
    id: personaId,
    envVars: request.request.envVars,
  });
  try {
    const resurrection = await bridge.resurrectStopped({ personaId });
    if (resurrection.failed.length > 0) {
      console.error(
        "[delegated-apply] resurrect_stopped_active_instances start failures",
        resurrection.failed,
      );
    }
    return mergeResurrectionIntoActivation(activation, resurrection);
  } catch (error) {
    console.error(
      "[delegated-apply] resurrect_stopped_active_instances failed after env apply",
      error,
    );
    return {
      ...activation,
      failed_start_count: activation.failed_start_count + 1,
    };
  }
}

export function applyDelegatedPersonaEnvVarsRequest(
  request: EnvRequest,
  personaId: string,
): Promise<DelegatedPersonaEnvVarsResult> {
  return applyDelegatedPersonaEnvVarsWith(request, personaId, {
    applyPatch: applyDelegatedPersonaEnvVars,
    resurrectStopped: resurrectStoppedActiveInstances,
  });
}
