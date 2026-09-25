//! Cleanup through the user's own Claude plan, via the unmodified Claude Code
//! CLI (personal use: the user's own sign-in to Anthropic's binary).
//!
//! The process is started when dictation starts, so its ~1–3 s startup
//! overlaps the user's speech; cleanup then only pays the model call.
//! Isolation: the token travels in `CLAUDE_CODE_OAUTH_TOKEN` (never argv),
//! `HOME`/`CLAUDE_CONFIG_DIR` point at a throwaway directory, and the CLI runs
//! with no tools, no MCP servers, no setting sources and thinking disabled
//! (`MAX_THINKING_TOKENS=0`; with thinking on, cleanup takes ~10 s).

use std::{
    collections::HashMap,
    process::Stdio,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use serde_json::{json, Value};
use tauri::AppHandle;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout},
};

use super::cleanup::{self, CleanupFailure, CleanupInput};

/// Output cap for one cleanup response stream.
const MAX_STREAM_BYTES: usize = 1024 * 1024;
/// Environment passed through to the CLI besides the isolated paths.
const PASSTHROUGH_ENV: &[&str] = &["USER", "LOGNAME", "LANG", "LC_ALL", "TMPDIR"];

/// A started Claude Code process waiting for the transcript.
pub(crate) struct Prewarmed {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// Holds the isolated HOME/config directory alive until the process ends.
    _home: tempfile::TempDir,
}

/// In-flight or finished launches keyed by dictation session. Storing the
/// launch task (not its result) means cleanup awaits the same process instead
/// of racing it, and discard aborts it; `kill_on_drop` ends the process.
type Launch = tauri::async_runtime::JoinHandle<Result<Prewarmed, CleanupFailure>>;

fn launches() -> &'static Mutex<HashMap<u64, Launch>> {
    static LAUNCHES: OnceLock<Mutex<HashMap<u64, Launch>>> = OnceLock::new();
    LAUNCHES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Resolves the account to use: the configured one or the default Claude
/// account. Only ready, unexpired token accounts are accepted.
fn resolve_account(app: &AppHandle, account_id: Option<&str>) -> Result<String, String> {
    let id = match account_id {
        Some(id) => id.to_string(),
        None => crate::subscription_accounts::defaults::selected_for(app, "claude")?
            .ok_or_else(|| "Choose a Claude account for dictation cleanup".to_string())?,
    };
    crate::subscription_accounts::defaults::validate_destination(app, &id)?;
    Ok(id)
}

/// The CLI arguments. Pure so tests can pin the isolation flags.
pub(crate) fn cli_args(model: &str, system_prompt: &str) -> Vec<String> {
    [
        "-p",
        "--model",
        model,
        "--system-prompt",
        system_prompt,
        "--tools",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
        "--setting-sources",
        "",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Environment for the CLI process. Pure so tests can pin that thinking is
/// off and the token never appears anywhere but its own variable.
pub(crate) fn cli_env(
    home: &std::path::Path,
    path: Option<String>,
    token: &str,
    passthrough: impl Fn(&str) -> Option<String>,
) -> Vec<(String, String)> {
    let home = home.to_string_lossy().into_owned();
    let mut env = vec![
        ("HOME".to_string(), home.clone()),
        ("CLAUDE_CONFIG_DIR".to_string(), home),
        ("CLAUDE_CODE_OAUTH_TOKEN".to_string(), token.to_string()),
        ("MAX_THINKING_TOKENS".to_string(), "0".to_string()),
        ("DISABLE_AUTOUPDATER".to_string(), "1".to_string()),
    ];
    if let Some(path) = path {
        env.push(("PATH".to_string(), path));
    }
    for key in PASSTHROUGH_ENV {
        if let Some(value) = passthrough(key) {
            env.push(((*key).to_string(), value));
        }
    }
    env
}

async fn spawn(
    app: &AppHandle,
    account_id: Option<&str>,
    model: &str,
    input: &CleanupInput,
) -> Result<Prewarmed, CleanupFailure> {
    // The system prompt (roster, vocabulary) leaves the process at launch, so
    // it passes the same outbound secret check as the cleanup call.
    cleanup::check_outbound(input)?;
    let unavailable = |message: String| CleanupFailure::Unavailable { message };
    let executable = crate::managed_agents::claude_cli::resolved_claude_cli()
        .ok_or_else(|| unavailable("Claude Code is not installed".into()))?;
    let account = resolve_account(app, account_id).map_err(unavailable)?;
    let account_home =
        crate::subscription_accounts::account_home(app, &account).map_err(unavailable)?;
    let token = crate::subscription_accounts::read_token(&account_home).map_err(unavailable)?;
    let home = tempfile::tempdir().map_err(|e| unavailable(e.to_string()))?;

    let mut command = tokio::process::Command::new(executable);
    command
        .env_clear()
        .current_dir(home.path())
        .args(cli_args(model, &cleanup::system_prompt(input)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let path = crate::managed_agents::readiness::cli_probe::augmented_path()
        .or_else(|| std::env::var("PATH").ok());
    for (key, value) in cli_env(home.path(), path, token.as_str(), |k| std::env::var(k).ok()) {
        command.env(key, value);
    }
    let mut child = command
        .spawn()
        .map_err(|e| unavailable(format!("could not start Claude Code: {e}")))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| unavailable("Claude Code stdin unavailable".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| unavailable("Claude Code stdout unavailable".into()))?;
    Ok(Prewarmed {
        child,
        stdin,
        stdout: BufReader::new(stdout),
        _home: home,
    })
}

/// Starts the CLI for `session` so its startup overlaps the user's speech.
/// Returns immediately; a launch failure resurfaces at cleanup time.
pub(crate) fn prewarm(
    app: AppHandle,
    session: u64,
    account_id: Option<String>,
    model: String,
    input: CleanupInput,
) {
    let launch = tauri::async_runtime::spawn(async move {
        spawn(&app, account_id.as_deref(), &model, &input).await
    });
    if let Ok(mut map) = launches().lock() {
        if let Some(previous) = map.insert(session, launch) {
            previous.abort();
        }
    }
}

/// Aborts a session's launch and drops (kills) its process, if any.
pub(crate) fn discard(session: u64) {
    if let Some(launch) = launches()
        .lock()
        .ok()
        .and_then(|mut map| map.remove(&session))
    {
        launch.abort();
    }
}

/// Extracts the final text from one stream-json line, if it is the result.
pub(crate) fn parse_result_line(line: &str) -> Option<Result<String, CleanupFailure>> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("result") {
        return None;
    }
    if value.get("is_error").and_then(Value::as_bool) == Some(true) {
        return Some(Err(CleanupFailure::Unavailable {
            message: "Claude Code reported an error".into(),
        }));
    }
    let text = value
        .get("result")
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    Some(if text.is_empty() {
        Err(CleanupFailure::BadResponse)
    } else {
        Ok(text)
    })
}

async fn read_result(stdout: &mut BufReader<ChildStdout>) -> Result<String, CleanupFailure> {
    let mut consumed = 0usize;
    let mut line = String::new();
    loop {
        line.clear();
        let read = stdout
            .read_line(&mut line)
            .await
            .map_err(|_| CleanupFailure::BadResponse)?;
        if read == 0 {
            return Err(CleanupFailure::BadResponse);
        }
        consumed += read;
        if consumed > MAX_STREAM_BYTES {
            return Err(CleanupFailure::BadResponse);
        }
        if let Some(result) = parse_result_line(&line) {
            return result;
        }
    }
}

/// Runs cleanup for `session`, using its prewarmed process when present.
pub(crate) async fn run(
    app: &AppHandle,
    session: u64,
    account_id: Option<&str>,
    model: &str,
    input: &CleanupInput,
    timeout: Duration,
) -> Result<String, CleanupFailure> {
    cleanup::check_outbound(input)?;
    let deadline = tokio::time::Instant::now() + timeout;
    let launch = launches()
        .lock()
        .ok()
        .and_then(|mut map| map.remove(&session));
    let mut process = match launch {
        Some(launch) => match tokio::time::timeout_at(deadline, launch).await {
            Err(_) => return Err(CleanupFailure::Timeout),
            Ok(Err(_)) => return Err(CleanupFailure::BadResponse),
            Ok(Ok(result)) => result?,
        },
        None => tokio::time::timeout_at(deadline, spawn(app, account_id, model, input))
            .await
            .unwrap_or(Err(CleanupFailure::Timeout))?,
    };
    let message = json!({
        "type": "user",
        "message": {"role": "user", "content": cleanup::user_message(&input.transcript)},
    });
    let exchange = async {
        process
            .stdin
            .write_all(format!("{message}\n").as_bytes())
            .await
            .map_err(|_| CleanupFailure::BadResponse)?;
        process
            .stdin
            .flush()
            .await
            .map_err(|_| CleanupFailure::BadResponse)?;
        read_result(&mut process.stdout).await
    };
    let outcome = tokio::time::timeout_at(deadline, exchange)
        .await
        .unwrap_or(Err(CleanupFailure::Timeout));
    let _ = process.child.start_kill();
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_runs_isolated_with_no_tools_or_settings() {
        let args = cli_args("claude-haiku-4-5", "SYSTEM");
        let pair = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .map(|i| args[i + 1].clone())
        };
        assert_eq!(pair("--model").as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(pair("--tools").as_deref(), Some(""));
        assert_eq!(pair("--setting-sources").as_deref(), Some(""));
        assert_eq!(
            pair("--mcp-config").as_deref(),
            Some(r#"{"mcpServers":{}}"#)
        );
        assert!(args.contains(&"--strict-mcp-config".to_string()));
    }

    #[test]
    fn thinking_is_off_and_token_stays_out_of_argv() {
        let home = std::path::Path::new("/tmp/iso");
        let env = cli_env(home, Some("/usr/bin".into()), "sk-ant-oat01-secret", |_| {
            None
        });
        let get = |k: &str| {
            env.iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("MAX_THINKING_TOKENS"), Some("0"));
        assert_eq!(get("HOME"), Some("/tmp/iso"));
        assert_eq!(get("CLAUDE_CONFIG_DIR"), Some("/tmp/iso"));
        assert_eq!(get("CLAUDE_CODE_OAUTH_TOKEN"), Some("sk-ant-oat01-secret"));
        assert!(get("ANTHROPIC_API_KEY").is_none());
        let args = cli_args("claude-haiku-4-5", "SYSTEM");
        assert!(!args.iter().any(|a| a.contains("sk-ant")));
    }

    #[test]
    fn parses_only_the_result_line() {
        assert!(parse_result_line(r#"{"type":"system","subtype":"init"}"#).is_none());
        assert!(parse_result_line("not json").is_none());
        assert_eq!(
            parse_result_line(
                r#"{"type":"result","is_error":false,"result":" Tell @[Ember] the build is green. "}"#
            ),
            Some(Ok("Tell @[Ember] the build is green.".into()))
        );
        assert!(matches!(
            parse_result_line(r#"{"type":"result","is_error":true,"result":"x"}"#),
            Some(Err(CleanupFailure::Unavailable { .. }))
        ));
        assert_eq!(
            parse_result_line(r#"{"type":"result","is_error":false,"result":"  "}"#),
            Some(Err(CleanupFailure::BadResponse))
        );
    }
}
