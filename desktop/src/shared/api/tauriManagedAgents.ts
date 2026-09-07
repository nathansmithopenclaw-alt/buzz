import {
  fromRawManagedAgent,
  invokeTauri,
  type RawManagedAgent,
} from "@/shared/api/tauri";
import type {
  ManagedAgent,
  ManagedAgentRuntimeStatus,
} from "@/shared/api/types";

export type ApplyManagedAgentAvatarResult = {
  applied: boolean;
  targetPubkey: string;
  personaId: string;
  instanceCount: number;
  reconciledCount: number;
};

export async function applyManagedAgentAvatarRequest(
  sourcePubkey: string,
  authorizationEvent: string,
): Promise<ApplyManagedAgentAvatarResult> {
  return invokeTauri("apply_managed_agent_avatar_request", {
    sourcePubkey,
    authorizationEvent,
  });
}

export type ManagementRequestDisposition =
  | "apply"
  | "draft"
  | "duplicate_applying"
  | "duplicate_applied"
  | "duplicate_draft"
  | "rejected"
  | "duplicate_failed"
  | "duplicate_dismissed";

export type RegisterManagementRequestResult = {
  disposition: ManagementRequestDisposition;
  auditChannelId?: string;
  result?: unknown;
};

export type RestoredManagementRequest = {
  requestId: string;
  sourcePubkey: string;
  payload: unknown;
  capability:
    | "agent_config"
    | "agent_creation"
    | "env_vars"
    | "global_config"
    | "teams";
  disposition: "draft" | "failed";
  result?: unknown;
};

const agentManagementStartupCutoff = new Date().toISOString();

export async function registerAgentManagementRequest(input: {
  sourcePubkey: string;
  requestId: string;
  capability:
    | "agent_config"
    | "agent_creation"
    | "env_vars"
    | "global_config"
    | "teams";
  intent: "apply" | "draft";
  payload: unknown;
}): Promise<RegisterManagementRequestResult> {
  return invokeTauri("agent_management_ledger", {
    command: { action: "register", ...input },
  });
}

export async function completeAgentManagementRequest(
  requestId: string,
  succeeded: boolean,
  result: unknown,
): Promise<void> {
  return invokeTauri("agent_management_ledger", {
    command: { action: "complete", requestId, succeeded, result },
  });
}

export async function beginOwnerReviewedAgentManagementRequest(
  requestId: string,
): Promise<void> {
  return invokeTauri("agent_management_ledger", {
    command: { action: "begin_owner_review", requestId },
  });
}

export async function restoreAgentManagementRequests(): Promise<
  RestoredManagementRequest[]
> {
  return invokeTauri("agent_management_ledger", {
    command: {
      action: "restore",
      startupCutoff: agentManagementStartupCutoff,
    },
  });
}

export async function dismissAgentManagementRequest(
  requestId: string,
): Promise<void> {
  return invokeTauri("agent_management_ledger", {
    command: { action: "dismiss", requestId },
  });
}

export async function publishAgentCreateSecretResponse(input: {
  requestId: string;
  sourcePubkey: string;
  agentPubkey: string;
  privateKeyNsec: string;
}): Promise<void> {
  return invokeTauri("publish_agent_management", {
    command: { action: "create_secret", ...input },
  });
}

export async function getManagedAgentNsec(pubkey: string): Promise<string> {
  return invokeTauri("get_managed_agent_nsec", { pubkey });
}

export async function publishAgentManagementAuditReceipt(input: {
  requestId: string;
  sourcePubkey: string;
  requesterName: string;
  capability:
    | "agent_config"
    | "agent_creation"
    | "env_vars"
    | "global_config"
    | "teams";
  target: string;
  succeeded: boolean;
}): Promise<void> {
  return invokeTauri("publish_agent_management", {
    command: { action: "audit", ...input },
  });
}

export async function publishAgentManagementDraftResult(input: {
  requestId: string;
  sourcePubkey: string;
}): Promise<void> {
  return invokeTauri("publish_agent_management", {
    command: { action: "draft", ...input },
  });
}

export async function publishAgentManagementApplyingResult(input: {
  requestId: string;
  sourcePubkey: string;
}): Promise<void> {
  return invokeTauri("publish_agent_management", {
    command: { action: "applying", ...input },
  });
}

export async function publishAgentManagementResult(input: {
  requestId: string;
  sourcePubkey: string;
  result: unknown;
}): Promise<void> {
  return invokeTauri("publish_agent_management", {
    command: { action: "result", ...input },
  });
}

export async function ensureManagedAgentForInbound(input: {
  pubkey: string;
  authorPubkey: string;
  isDm: boolean;
}): Promise<{ pubkey: string; spawned: boolean; reason: string }> {
  return invokeTauri("ensure_managed_agent_for_inbound", input);
}

export type FleetVisibilityRole = "host" | "observer";

export type FleetVisibilitySettings = {
  enabled: boolean;
  role: FleetVisibilityRole;
  needsChoice: boolean;
  hostId?: string;
  hostName?: string;
};

export function getFleetVisibilitySettings(): Promise<FleetVisibilitySettings> {
  return invokeTauri("get_fleet_visibility_settings");
}

export function listKeyedManagedAgentPubkeys(): Promise<string[]> {
  return invokeTauri("list_keyed_managed_agent_pubkeys");
}

export function setFleetVisibilityEnabled(
  enabled: boolean,
): Promise<FleetVisibilitySettings> {
  return invokeTauri("set_fleet_visibility_enabled", { enabled });
}

export function setFleetVisibilityRole(
  role: FleetVisibilityRole,
): Promise<FleetVisibilitySettings> {
  return invokeTauri("set_fleet_visibility_role", { role });
}

export async function resurrectStoppedActiveInstances(input: {
  personaId?: string;
  pubkey?: string;
}): Promise<{ started: string[]; failed: string[] }> {
  return invokeTauri("resurrect_stopped_active_instances", input);
}

export async function startManagedAgent(
  pubkey: string,
  options?: {
    /** Tenant scope captured by the caller before its first await; the
     * backend fails closed before any spawn/deploy side effect when the
     * active community no longer matches. */
    expectedRelayUrl?: string;
    /** Signer identity captured with the relay scope; the backend fails
     * closed when the active workspace identity no longer matches. */
    expectedSignerPubkey?: string;
    /** Unix-seconds replay floor for a publish-first mention send: the
     * spawned harness's first REQ replays at least back to this moment, so
     * the already-published triggering message lands in its window however
     * long the spawn takes. Local spawns receive it as process env; provider
     * deploys carry it in the payload's launch.policy_env. */
    replayFloorUnix?: number;
  },
): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>("start_managed_agent", {
    pubkey,
    expectedRelayUrl: options?.expectedRelayUrl ?? null,
    expectedSignerPubkey: options?.expectedSignerPubkey ?? null,
    replayFloorUnix: options?.replayFloorUnix ?? null,
  });
  return fromRawManagedAgent(response);
}

export async function stopManagedAgent(pubkey: string): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>("stop_managed_agent", {
    pubkey,
  });
  return fromRawManagedAgent(response);
}

export async function setManagedAgentStartOnAppLaunch(
  pubkey: string,
  startOnAppLaunch: boolean,
): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>(
    "set_managed_agent_start_on_app_launch",
    {
      pubkey,
      startOnAppLaunch,
    },
  );
  return fromRawManagedAgent(response);
}

export async function setManagedAgentAutoRestart(
  pubkey: string,
  autoRestartOnConfigChange: boolean,
): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>(
    "set_managed_agent_auto_restart",
    {
      pubkey,
      autoRestartOnConfigChange,
    },
  );
  return fromRawManagedAgent(response);
}

export async function setManagedAgentSessionResidencyPolicy(
  pubkey: string,
  idlePoolSleepSeconds: number | null,
  maxTurnsPerSession: number | null,
): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>(
    "set_managed_agent_session_residency_policy",
    {
      pubkey,
      idlePoolSleepSeconds,
      maxTurnsPerSession,
    },
  );
  return fromRawManagedAgent(response);
}

export async function listManagedAgentRuntimes(): Promise<
  ManagedAgentRuntimeStatus[]
> {
  return invokeTauri<ManagedAgentRuntimeStatus[]>(
    "list_managed_agent_runtimes",
  );
}

export async function startManagedAgentRuntime(
  pubkey: string,
  relayUrl: string,
): Promise<ManagedAgentRuntimeStatus> {
  return invokeTauri("start_managed_agent_runtime", { pubkey, relayUrl });
}

export async function stopManagedAgentRuntime(
  pubkey: string,
  relayUrl: string,
): Promise<ManagedAgentRuntimeStatus> {
  return invokeTauri("stop_managed_agent_runtime", { pubkey, relayUrl });
}

export async function restartManagedAgentRuntime(
  pubkey: string,
  relayUrl: string,
): Promise<ManagedAgentRuntimeStatus> {
  return invokeTauri("restart_managed_agent_runtime", { pubkey, relayUrl });
}

export async function putManagedAgentRuntimeLifecycle(
  outerPubkey: string,
  payload: unknown,
): Promise<ManagedAgentRuntimeStatus> {
  return invokeTauri("put_managed_agent_runtime_lifecycle", {
    outerPubkey,
    payload,
  });
}

export async function reconcileManagedAgentRuntimes(
  communities: readonly { relayUrl: string }[],
): Promise<ManagedAgentRuntimeStatus[]> {
  return invokeTauri("reconcile_managed_agent_runtimes", { communities });
}
