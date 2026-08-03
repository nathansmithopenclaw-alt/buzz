//! Persisted runtime identity helpers shared by load, migration, and UI summary paths.

use super::{
    custom_harnesses::lookup_loaded_harness_by_id, known_acp_runtime, known_acp_runtime_exact,
    preset_harness_definitions, ManagedAgentRecord,
};

/// Resolve a persisted runtime id to the command its instance snapshot should
/// carry. Presets are available before the loaded-harness registry is warmed at
/// boot; custom harnesses join once that registry is ready.
pub(crate) fn runtime_primary_command(id: &str) -> Option<String> {
    if let Some(command) = known_acp_runtime_exact(id).and_then(|runtime| runtime.commands.first())
    {
        return Some((*command).to_string());
    }
    if let Some(definition) = lookup_loaded_harness_by_id(id) {
        return Some(definition.command.clone());
    }
    preset_harness_definitions()
        .into_iter()
        .find(|definition| definition.id == id)
        .map(|definition| definition.command)
}

/// Claude-account UI is owned by the persisted runtime identity. A stale
/// legacy command snapshot must never make an explicitly non-Claude instance
/// look Claude-capable. Runtime-less legacy records retain the descriptor
/// fallback until their runtime is materialized.
pub(crate) fn record_supports_claude_accounts(
    record: &ManagedAgentRecord,
    descriptor_command: &str,
) -> bool {
    supports_claude_accounts(record.runtime.as_deref(), descriptor_command)
}

fn supports_claude_accounts(runtime_id: Option<&str>, descriptor_command: &str) -> bool {
    match runtime_id {
        Some(runtime_id) => runtime_id == "claude",
        None => known_acp_runtime(descriptor_command).is_some_and(|runtime| runtime.id == "claude"),
    }
}

#[cfg(test)]
mod tests {
    use super::supports_claude_accounts;

    #[test]
    fn claude_account_support_respects_explicit_record_runtime() {
        assert!(!supports_claude_accounts(Some("grok"), "claude-agent-acp"));
        assert!(supports_claude_accounts(Some("claude"), "claude-agent-acp"));
        assert!(supports_claude_accounts(None, "claude-agent-acp"));
    }
}
