import { invokeTauri } from "@/shared/api/tauri";
import type {
  AgentManagementDelegationSubjects,
  GlobalAgentConfig,
  GlobalAgentConfigSaveResult,
} from "@/shared/api/types";

/**
 * Read the current global agent configuration defaults.
 *
 * Returns an empty default if the file has not been written yet.
 */
export async function getGlobalAgentConfig(): Promise<GlobalAgentConfig> {
  return invokeTauri<GlobalAgentConfig>("get_global_agent_config");
}

/**
 * Validate and persist a new global agent configuration.
 *
 * The backend strips empty env values (empty = "inherit"), validates key
 * shape and reserved-key rules, restarts running local agents whose effective
 * env changed, and returns the saved config with a restart count.
 *
 * Throws a string error message on validation failure.
 */
export async function setGlobalAgentConfig(
  config: GlobalAgentConfig,
): Promise<GlobalAgentConfigSaveResult> {
  return invokeTauri<GlobalAgentConfigSaveResult>("set_global_agent_config", {
    config,
  });
}

export type DelegatedGlobalAgentConfigPatch = {
  provider?: string;
  model?: string;
  preferredRuntime?: string;
  envVars: Record<string, string>;
};

/** Atomically overlay delegated fields onto the latest durable config. */
export async function applyDelegatedGlobalAgentConfig(
  patch: DelegatedGlobalAgentConfigPatch,
): Promise<GlobalAgentConfigSaveResult> {
  return invokeTauri<GlobalAgentConfigSaveResult>(
    "apply_delegated_global_agent_config",
    { patch },
  );
}

/**
 * Persist the standing-delegation grant against the latest durable config.
 *
 * Independent of Agent-defaults Save: unsaved provider/model edits are not
 * part of this write. `null` or an all-`none` subject map revokes the grant.
 * The backend ensures the private audit channel before signing.
 */
export async function setAgentManagementDelegation(
  capabilities: AgentManagementDelegationSubjects | null,
): Promise<GlobalAgentConfigSaveResult> {
  return invokeTauri<GlobalAgentConfigSaveResult>(
    "set_agent_management_delegation",
    { capabilities },
  );
}
