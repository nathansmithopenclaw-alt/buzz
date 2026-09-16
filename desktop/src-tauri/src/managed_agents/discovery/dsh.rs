//! Capabilities of the private DeepSeek preset. Installation stays preset-owned.
use super::KnownAcpRuntime;
pub(super) const DSH_RUNTIME: KnownAcpRuntime = KnownAcpRuntime {
    id: "deepseek",
    label: "DeepSeek Harness",
    commands: &["dsh"],
    aliases: &[],
    avatar_url: "",
    mcp_command: None,
    mcp_hooks: false,
    underlying_cli: None,
    cli_install_commands: &[],
    cli_install_commands_windows: &[],
    adapter_install_commands: &[],
    cli_install_instructions_url: "https://github.com/block/buzz",
    adapter_install_instructions_url: "https://github.com/block/buzz",
    cli_install_hint: "Use the pinned DeepSeek harness with the Buzz ACP profile.",
    adapter_install_hint: "",
    skill_dir: None,
    supports_acp_model_switching: false,
    model_env_var: Some("DSH_MODEL"),
    provider_env_var: Some("DSH_PROVIDER"),
    provider_locked: false,
    default_env: &[],
    config_file_path: None,
    config_file_format: None,
    supports_acp_native_config: false,
    thinking_env_var: None,
    effort_model_qualifier: false,
    effort_normalization: None,
    effort_accepted_values: None,
    max_tokens_env_var: None,
    context_limit_env_var: Some("DSH_CONTEXT_WINDOW"),
    max_rounds_env_var: None,
    required_normalized_fields: &["model", "provider"],
    login_hint: None,
    auth_probe_args: None,
};

pub(super) const PRIME_RUNTIME: KnownAcpRuntime = KnownAcpRuntime {
    id: "prime",
    label: "Prime Agent",
    commands: &["prime-agent"],
    aliases: &["primeagent"],
    model_env_var: Some("BUZZ_PRIME_MODEL"),
    provider_env_var: Some("BUZZ_PRIME_PROVIDER"),
    context_limit_env_var: None,
    required_normalized_fields: &[],
    ..DSH_RUNTIME
};

#[cfg(test)]
mod tests {
    #[test]
    fn dsh_metadata_does_not_introduce_an_empty_default_avatar() {
        assert!(super::super::managed_agent_avatar_url("dsh").is_none());
    }
}
