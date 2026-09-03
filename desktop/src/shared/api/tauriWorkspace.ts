import { invokeTauri } from "@/shared/api/tauri";

export async function applyCommunity(
  relayUrl: string,
  nsec?: string,
  token?: string,
  reposDir?: string,
  agentManagedProfiles?: boolean,
  fleetVisibility?: boolean,
  threadScopedAcpSessions?: boolean,
  agentSessionResidency?: boolean,
  answeredEventWatermark?: boolean,
): Promise<void> {
  await invokeTauri("apply_workspace", {
    relayUrl,
    nsec: nsec ?? null,
    token: token ?? null,
    reposDir: reposDir ?? null,
    experiments: {
      agentManagedProfiles: agentManagedProfiles ?? false,
      fleetVisibility: fleetVisibility ?? false,
      threadScopedAcpSessions: threadScopedAcpSessions ?? false,
      agentSessionResidency: agentSessionResidency ?? false,
      answeredEventWatermark: answeredEventWatermark ?? false,
    },
  });
}

export const setAgentManagedProfiles = (enabled: boolean) =>
  invokeTauri("set_agent_managed_profiles", { enabled });

export const setThreadScopedAcpSessions = (enabled: boolean) =>
  invokeTauri("set_thread_scoped_acp_sessions", { enabled });

export const setAgentSessionResidency = (enabled: boolean) =>
  invokeTauri("set_agent_session_residency", { enabled });

export const setAnsweredEventWatermark = (enabled: boolean) =>
  invokeTauri("set_answered_event_watermark", { enabled });
