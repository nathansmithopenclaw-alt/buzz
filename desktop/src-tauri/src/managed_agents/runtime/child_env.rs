use std::{collections::BTreeMap, process::Command};

const ACP_EFFORT_KEY: &str = "BUZZ_ACP_THINKING_EFFORT";
const ACP_EFFORT_DEFAULT_KEY: &str = "BUZZ_ACP_THINKING_EFFORT_IS_DEFAULT";

pub(super) fn child_rust_log_filter() -> String {
    child_rust_log_filter_from(std::env::var("RUST_LOG").ok().as_deref())
}

pub(super) fn apply_thinking_effort_provenance(
    command: &mut Command,
    persona_env: &BTreeMap<String, String>,
    agent_env: &BTreeMap<String, String>,
    global_env: &BTreeMap<String, String>,
) {
    // Never inherit or accept a forged marker. Desktop sets it only when the
    // effective generic effort came from the global default layer.
    command.env_remove(ACP_EFFORT_DEFAULT_KEY);
    if super::super::env_vars::env_var_comes_from_global_default(
        ACP_EFFORT_KEY,
        persona_env,
        agent_env,
        global_env,
    ) {
        command.env(ACP_EFFORT_DEFAULT_KEY, "true");
    }
}

pub(super) fn child_rust_log_filter_from(existing: Option<&str>) -> String {
    let mut directives = existing.unwrap_or_default().trim().to_string();
    for required in ["buzz_acp=info", "pool::model=info"] {
        let target = required.split('=').next().unwrap_or(required);
        if !directives
            .split(',')
            .any(|entry| entry.trim().starts_with(target))
        {
            if !directives.is_empty() {
                directives.push(',');
            }
            directives.push_str(required);
        }
    }
    directives
}
