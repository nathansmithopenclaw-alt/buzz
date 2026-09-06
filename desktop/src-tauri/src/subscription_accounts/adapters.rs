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
        SubscriptionAccountCapability { provider: "anthropic", harness: "claude", supported: false, token_supported: cfg!(unix), reason: Some("Claude accounts use long-lived tokens") },
        SubscriptionAccountCapability { provider: "openai", harness: "codex", supported: cfg!(unix), token_supported: false, reason: if cfg!(unix) { None } else { Some("Login process containment is not implemented on this platform") } },
        SubscriptionAccountCapability { provider: "xai", harness: "grok", supported: false, token_supported: false, reason: Some("Grok 1.0.13 exposes login/logout but no documented subscription status check; configuration preservation also needs validation") },
    ]
}

pub(super) fn provider(harness: &str) -> Result<&'static str, String> {
    // Existing login consumers retain their guarded runtime path during migration.
    // New login and reconnect use login_provider below.
    if harness == "claude" && cfg!(target_os = "macos") {
        return Ok("anthropic");
    }
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

pub(super) fn login_provider(harness: &str) -> Result<&'static str, String> {
    if harness == "claude" {
        return Err("Claude subscription sign-in has been retired. Add a Claude long-lived token in Settings → AI.".into());
    }
    provider(harness)
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

#[cfg(test)]
mod login_tests {
    use super::*;

    #[test]
    fn claude_offers_tokens_and_rejects_login_without_disabling_existing_cleanup() {
        let claude = capabilities()
            .into_iter()
            .find(|c| c.harness == "claude")
            .unwrap();
        assert!(!claude.supported);
        assert_eq!(claude.token_supported, cfg!(unix));
        assert!(login_provider("claude")
            .unwrap_err()
            .contains("long-lived token"));
        assert_eq!(args("claude", "logout").unwrap(), ["auth", "logout"]);
    }

    #[test]
    fn openai_login_remains_available_on_supported_platforms() {
        assert_eq!(login_provider("codex").is_ok(), cfg!(unix));
    }
}
