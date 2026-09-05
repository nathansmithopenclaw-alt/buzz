//! Harness commands own credential storage, refresh, login and logout.
use std::{
    path::Path,
    process::Command,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use serde::Serialize;

use super::process;

#[derive(Serialize)]
pub(crate) struct SubscriptionAccountCapability {
    pub provider: &'static str,
    pub harness: &'static str,
    pub supported: bool,
    pub token_supported: bool,
    pub reason: Option<&'static str>,
}

pub(super) fn capabilities() -> Vec<SubscriptionAccountCapability> {
    vec![
        SubscriptionAccountCapability { provider: "anthropic", harness: "claude", supported: cfg!(target_os = "macos"), token_supported: cfg!(unix), reason: if cfg!(target_os = "macos") { None } else { Some("Credential-only isolation currently requires macOS Keychain") } },
        SubscriptionAccountCapability { provider: "openai", harness: "codex", supported: cfg!(unix), token_supported: false, reason: if cfg!(unix) { None } else { Some("Login process containment is not implemented on this platform") } },
        SubscriptionAccountCapability { provider: "xai", harness: "grok", supported: false, token_supported: false, reason: Some("Grok 1.0.13 exposes login/logout but no documented subscription status check; configuration preservation also needs validation") },
    ]
}

pub(super) fn provider(harness: &str) -> Result<&'static str, String> {
    let capability = capabilities()
        .into_iter()
        .find(|entry| entry.harness == harness)
        .ok_or("Unknown subscription harness")?;
    if !capability.supported {
        return Err(capability
            .reason
            .unwrap_or("Unsupported subscription harness")
            .into());
    }
    Ok(capability.provider)
}

/// Authentication commands run with a small environment, so login cannot
/// silently select an inherited API key, cloud provider, or ambient OAuth token.
pub(super) fn isolated_command(
    executable: &Path,
    harness: &str,
    home: &Path,
    args: &[&str],
    timeout: Duration,
) -> Command {
    let mut command = process::command(executable, args, timeout);
    command.env_clear();
    for key in [
        "HOME",
        "USER",
        "LOGNAME",
        "PATH",
        "TMPDIR",
        "LANG",
        "LC_ALL",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.current_dir(home);
    match harness {
        "claude" => {
            command
                .env("CLAUDE_CONFIG_DIR", home)
                .env("CLAUDE_SECURESTORAGE_CONFIG_DIR", home);
        }
        "codex" => {
            command.env("CODEX_HOME", home);
        }
        _ => {}
    }
    command
}

fn args<'a>(harness: &str, action: &'a str) -> Result<Vec<&'a str>, String> {
    match (harness, action) {
        ("claude", "login" | "status" | "logout") => Ok(vec!["auth", action]),
        ("codex", "login") => Ok(vec!["-c", "cli_auth_credentials_store=\"file\"", "login"]),
        ("codex", "status") => Ok(vec![
            "-c",
            "cli_auth_credentials_store=\"file\"",
            "login",
            "status",
        ]),
        ("codex", "logout") => Ok(vec!["-c", "cli_auth_credentials_store=\"file\"", "logout"]),
        _ => Err("Unsupported account operation".into()),
    }
}

pub(super) fn operation(
    executable: &Path,
    harness: &str,
    home: &Path,
    action: &str,
    cancelled: &Arc<AtomicBool>,
) -> Result<(), String> {
    let timeout = Duration::from_secs(if action == "login" { 300 } else { 20 });
    let args = args(harness, action)?;
    let command = isolated_command(executable, harness, home, &args, timeout);
    let output = process::run(command, timeout, cancelled, action == "status")?;
    if action == "status" {
        verify_status(harness, &output)?;
    }
    Ok(())
}

pub(super) fn verify_status(harness: &str, output: &[u8]) -> Result<(), String> {
    let subscription = match harness {
        "claude" => serde_json::from_slice::<serde_json::Value>(output)
            .ok()
            .is_some_and(|value| {
                value.get("loggedIn").and_then(serde_json::Value::as_bool) == Some(true)
                    && value.get("authMethod").and_then(serde_json::Value::as_str)
                        == Some("claude.ai")
            }),
        "codex" => std::str::from_utf8(output).ok().is_some_and(|text| {
            text.lines()
                .any(|line| line.trim() == "Logged in using ChatGPT")
        }),
        _ => false,
    };
    if !subscription {
        return Err("Provider did not confirm subscription authentication; reconnect with a subscription account".into());
    }
    Ok(())
}
