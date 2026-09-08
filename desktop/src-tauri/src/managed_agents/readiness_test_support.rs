//! Shared environment fixtures for readiness tests.

use super::EffectiveAgentEnv;
use crate::managed_agents::discovery::known_acp_runtime_exact;
use std::collections::BTreeMap;

/// Build a minimal `EffectiveAgentEnv` with the given env map and command.
pub(super) fn make_env(command: &str, env: BTreeMap<String, String>) -> EffectiveAgentEnv {
    let runtime = known_acp_runtime_exact(command);
    EffectiveAgentEnv {
        env,
        config_file_path: runtime.and_then(|r| r.config_file_path),
        effective_command: command.to_string(),
    }
}

pub(super) fn env_with(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}
