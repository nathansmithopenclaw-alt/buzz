//! Per-agent Codex projection. Hosted agents never enter this module.
use super::{qualification::Protocol, runtime::write_private, ModelConnection};
use buzz_harness_config::codex;
use std::{path::Path, process::Command};

pub(super) fn apply(
    root: &Path,
    command: &mut Command,
    connection: &ModelConnection,
    model: &str,
    key: Option<&str>,
    agent: &str,
) -> Result<(), String> {
    let cli = command
        .get_envs()
        .find(|(key, _)| *key == "CODEX_PATH")
        .and_then(|(_, value)| value)
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("CODEX_PATH").map(std::path::PathBuf::from))
        .or_else(|| crate::managed_agents::resolve_command("codex"))
        .ok_or(
            "Saved-connection Codex requires qualified CLI 0.144.0; installed version is unknown",
        )?;
    let path = command
        .get_envs()
        .find(|(key, _)| *key == "PATH")
        .and_then(|(_, value)| value)
        .and_then(|value| value.to_str());
    codex::validate_version(buzz_harness_config::cli_version::probe(&cli, path))?;
    command.env("CODEX_PATH", cli);
    let qualified = connection
        .qualification
        .as_ref()
        .ok_or("Test this model connection in Settings before starting the agent")?;
    if !qualified
        .check(model, Protocol::Responses)
        .is_some_and(|check| check.tool_round_trip)
    {
        return Err("This connection has not passed Responses tool qualification".into());
    }
    let context = qualified
        .context_window
        .filter(|size| *size >= 1024)
        .ok_or("This connection has no qualified context window")?;
    let cwd = command
        .get_current_dir()
        .ok_or("Codex connection requires an explicit workspace")?;
    buzz_harness_config::codex_project::validate(cwd)?;
    let home = super::runtime_home::prepare(root, agent, connection, model)?;
    codex::clear_shell_snapshots(&home)?;
    let catalog = home.join("models.json");
    write_private(
        &catalog,
        &serde_json::to_vec(&codex::catalog(model, context)).map_err(|e| e.to_string())?,
    )?;
    write_private(
        &home.join("config.toml"),
        codex::home_config(&connection.base_url, &catalog)?.as_bytes(),
    )?;
    command.env("CODEX_HOME", &home);
    command.env("CODEX_SQLITE_HOME", &home);
    let mut overrides = codex::overrides(model, context);
    let mode = command
        .get_envs()
        .find(|(key, _)| *key == "INITIAL_AGENT_MODE")
        .and_then(|(_, value)| value)
        .and_then(|value| value.to_str())
        .map(str::to_owned)
        .or_else(|| std::env::var("INITIAL_AGENT_MODE").ok());
    if mode.as_deref() == Some("agent-full-access") {
        overrides["sandbox_mode"] = serde_json::json!("danger-full-access");
        overrides["approval_policy"] = serde_json::json!("never");
    }
    overrides["model_catalog_json"] = serde_json::json!(catalog);
    command.env("CODEX_CONFIG", overrides.to_string());
    command.env(buzz_harness_config::CODEX_SESSION_ONLY_ENV, "1");
    command.env(codex::PROVIDER_KEY_ENV, key.unwrap_or("local-no-auth"));
    command.env(
        "BUZZ_ACP_PROVIDER_CONFIG",
        serde_json::json!({
            "providerId":"openai","apiType":"openai","baseUrl":connection.base_url,"headers":{}
        })
        .to_string(),
    );
    Ok(())
}

#[cfg(test)]
#[path = "native_codex_tests.rs"]
mod tests;
