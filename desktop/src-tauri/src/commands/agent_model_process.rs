use std::{collections::BTreeMap, path::PathBuf};

use crate::managed_agents::{
    build_buzz_agent_provider_defaults, claude_config::apply_claude_model_env,
    default_agent_workdir, known_acp_runtime, redact_env_values_in, AgentModelsResponse,
};

use super::agent_models::normalize_agent_models;

pub(super) async fn run_agent_models_command(
    resolved_acp: PathBuf,
    agent_command: String,
    agent_args: Vec<String>,
    persisted_model: Option<String>,
    merged_env: BTreeMap<String, String>,
) -> Result<AgentModelsResponse, String> {
    // Clone the env map for redaction below — `merged_env` is moved
    // into the spawn_blocking closure and we still need the values to
    // scrub any user-supplied secrets that the child surfaces in stderr.
    let env_for_redaction = merged_env.clone();
    let selected_model_for_command = persisted_model.clone();
    let agent_command_for_response = agent_command.clone();

    // Use spawn_blocking because the desktop Tauri crate doesn't enable
    // tokio's `process` feature. std::process::Command is synchronous
    // but fine for a short-lived subprocess (~2-5s).
    let output = tokio::task::spawn_blocking(move || {
        let mut cmd = agent_models_command(
            &resolved_acp,
            &agent_command,
            &agent_args,
            selected_model_for_command.as_deref(),
            &merged_env,
        )?;
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .map_err(|e| format!("failed to spawn buzz-acp models: {e}"))
    })
    .await
    .map_err(|e| format!("model discovery task failed: {e}"))?
    .map_err(|e: String| e)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Scrub any user-supplied env values before surfacing stderr to
        // the frontend — persona/agent env_vars may carry API keys that
        // a failing child process echoed back.
        let stderr_redacted = redact_env_values_in(stderr.as_ref(), &env_for_redaction);
        return Err(format!(
            "buzz-acp models failed (exit {}): {stderr_redacted}",
            output.status.code().unwrap_or(-1)
        ));
    }

    let raw: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("failed to parse model JSON: {e}"))?;

    Ok(agent_models_response(
        &raw,
        persisted_model,
        &agent_command_for_response,
    ))
}

/// Normalize the discovery output for the frontend.
///
/// When the saved model was selected at startup, the probed session's current
/// model is that selection rather than the harness default, so it must not be
/// reported as the model an unset agent would run.
fn agent_models_response(
    raw: &serde_json::Value,
    persisted_model: Option<String>,
    agent_command: &str,
) -> AgentModelsResponse {
    let startup_selected_model = selects_model_at_startup(agent_command)
        && persisted_model
            .as_deref()
            .is_some_and(|model| !model.trim().is_empty());
    let mut response = normalize_agent_models(raw, persisted_model);
    if startup_selected_model {
        response.agent_default_model = None;
    }
    response
}

/// Whether the runtime takes its model at process start (`ANTHROPIC_MODEL`)
/// rather than through an ACP model switch. Local Claude Code is the only
/// such runtime; the predicate mirrors the spawn-time A1 authority in
/// `spawn_agent_child`. Spawn identifies the runtime from its unresolved
/// command, but discovery receives the resolved adapter path, which on Windows
/// is an npm `.cmd`/`.bat` shim that runtime identity does not strip.
fn selects_model_at_startup(agent_command: &str) -> bool {
    let command = agent_command.trim();
    let lower = command.to_ascii_lowercase();
    let command = [".cmd", ".bat"]
        .iter()
        .find(|shim| lower.ends_with(*shim))
        .map_or(command, |shim| &command[..command.len() - shim.len()]);
    known_acp_runtime(command).is_some_and(|runtime| runtime.id == "claude")
}

/// Build the `buzz-acp models --json` discovery command.
///
/// Discovery must select the saved model the way a launched agent does, so it
/// reports the options that agent's sessions advertise. Claude Code takes its
/// model from `ANTHROPIC_MODEL` alone (the A1 contract), so it gets the same
/// `apply_claude_model_env` write that spawn performs. An ACP model switch
/// (`--model`) is a different path: claude-agent-acp rejects any id missing
/// from its settings allowlist, and the switch always runs the adapter's
/// `setModel`, which can fail an account check (Fable on a credits-gated
/// login) that a launch skips when the CLI already runs the saved id. Saved
/// models that launch fine therefore failed discovery. Every other runtime
/// keeps the explicit switch.
fn agent_models_command(
    resolved_acp: &std::path::Path,
    agent_command: &str,
    agent_args: &[String],
    selected_model: Option<&str>,
    merged_env: &BTreeMap<String, String>,
) -> Result<std::process::Command, String> {
    let selected_model = selected_model
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let startup_model = selects_model_at_startup(agent_command);
    let mut cmd = std::process::Command::new(resolved_acp);
    if let Some(home) = default_agent_workdir() {
        cmd.current_dir(home);
    }
    // Inject the same augmented PATH used for launched agents and CLI
    // probes: managed Node/npm dirs, exe-parent sidecars, login-shell
    // PATH, and (Windows) the inherited process PATH. login_shell_path()
    // alone is always None on Windows, which left this child with no
    // managed Node dirs — the ACP adapter's `.cmd` shims then failed with
    // `'node' is not recognized` and the model dropdown stayed empty.
    if let Some(ref path) = crate::managed_agents::readiness::cli_probe::augmented_path() {
        cmd.env("PATH", path);
    }
    cmd.arg("models")
        .arg("--json")
        .env("BUZZ_ACP_AGENT_COMMAND", agent_command)
        .env("BUZZ_ACP_AGENT_ARGS", agent_args.join(","));
    if let Some(model) = selected_model.filter(|_| !startup_model) {
        cmd.arg("--model").arg(model);
    }
    if let Some(meta) = known_acp_runtime(agent_command) {
        for (key, value) in meta.default_env {
            if std::env::var(key).is_err() {
                cmd.env(key, value);
            }
        }
    }
    // Mirror runtime spawn: internal builds may bake provider/model
    // defaults. User-provided env below still wins.
    build_buzz_agent_provider_defaults(&mut cmd);
    // User env layering — written LAST so it overrides any Buzz-set env above.
    for (k, v) in merged_env {
        cmd.env(k, v);
    }
    // Written after the user env, as at spawn, so the saved model wins over
    // a hand-set ANTHROPIC_MODEL and an unset model clears one.
    if startup_model {
        apply_claude_model_env(&mut cmd, selected_model);
    }
    // Demo identity is authoritative and must win over ambient/user env.
    crate::build_identity::apply_demo_config_home(&mut cmd)?;
    crate::managed_agents::configure_runtime_cli(&mut cmd, known_acp_runtime(agent_command));
    crate::util::configure_no_window(&mut cmd);
    Ok(cmd)
}

#[cfg(test)]
#[path = "agent_model_process_tests.rs"]
mod tests;
