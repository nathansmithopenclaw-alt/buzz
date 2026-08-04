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
  | "duplicate_failed"
  | "duplicate_dismissed";

export type RegisterManagementRequestResult = {
  disposition: ManagementRequestDisposition;
  auditChannelId?: string;
  result?: unknown;
};

export type PendingManagementRequest = {
  requestId: string;
  sourcePubkey: string;
  payload: unknown;
};

export async function registerAgentManagementRequest(input: {
  sourcePubkey: string;
  requestId: string;
  capability: "agent_config" | "agent_creation" | "teams";
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

export async function listPendingAgentManagementRequests(): Promise<
  PendingManagementRequest[]
> {
  return invokeTauri("agent_management_ledger", {
    command: { action: "list_pending" },
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
  capability: "agent_config" | "agent_creation" | "teams";
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

export async function publishAgentManagementResult(input: {
  requestId: string;
  sourcePubkey: string;
  result: unknown;
}): Promise<void> {
  return invokeTauri("publish_agent_management", {
    command: { action: "result", ...input },
  });
}

export async function startManagedAgent(pubkey: string): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>("start_managed_agent", {
    pubkey,
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
