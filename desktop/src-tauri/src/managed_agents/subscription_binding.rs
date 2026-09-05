//! Local account selection is authoritative even when the editing experiment is off.
use super::{ManagedAgentRecord, ManagedAgentRuntimeLifecycle};
use crate::{app_state::AppState, subscription_accounts};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

/// The lease pins an actual spawned child to one selected account generation.
#[derive(Debug)]
pub struct RuntimeBinding {
    pub account_id: String,
    pub generation: u64,
    pub(crate) _lease: Option<subscription_accounts::SubscriptionLease>,
}

pub(crate) fn account_harness(runtime_id: Option<&str>) -> Option<&'static str> {
    match runtime_id {
        Some("claude") => Some("claude"),
        Some("codex") => Some("codex"),
        Some("grok") => Some("grok"),
        _ => None,
    }
}

/// Apply after every user/legacy environment layer, immediately before spawn.
pub(crate) fn configure(
    app: &AppHandle,
    record: &ManagedAgentRecord,
    harness: Option<&str>,
    provider: Option<&str>,
    command: &mut Command,
) -> Result<Option<RuntimeBinding>, String> {
    // Do not inherit a binding from Desktop's own launch environment.
    for name in [
        "BUZZ_ACP_SUBSCRIPTION_KIND",
        "BUZZ_ACP_SUBSCRIPTION_ACCOUNT_ID",
        "BUZZ_ACP_SUBSCRIPTION_ACCOUNT_GENERATION",
        "BUZZ_ACP_SUBSCRIPTION_HARNESS",
        "BUZZ_ACP_SUBSCRIPTION_HOME",
        "BUZZ_ACP_SUBSCRIPTION_CONFIG_HOME",
    ] {
        command.env_remove(name);
    }
    let Some(id) = record.subscription_account_id.as_deref() else {
        return Ok(None);
    };
    if record.backend != super::BackendKind::Local {
        return Err("Subscription accounts require a local harness".into());
    }
    let harness = harness.ok_or("This harness does not support subscription accounts")?;
    let expected = match harness {
        "claude" => "anthropic",
        "codex" => "openai",
        _ => {
            return Err("This subscription adapter is not verified for managed runtime use".into())
        }
    };
    if provider.is_some_and(|p| !p.trim().is_empty() && p != expected) {
        return Err("Selected provider conflicts with the subscription account".into());
    }
    let _guard = subscription_accounts::lifecycle_lock()
        .lock()
        .map_err(|e| e.to_string())?;
    let account = subscription_accounts::load_account(app, id)?;
    let home = subscription_accounts::account_home(app, id)?;
    let lease = if account.kind == "token" {
        if harness != "claude"
            || account.harness != harness
            || account.state != "ready"
            || account.nickname.is_none()
        {
            return Err("Token account is not ready; renew it in Settings".into());
        }
        if account
            .expires_at
            .is_none_or(|at| at <= chrono::Utc::now().timestamp())
        {
            return Err("Token account has expired; renew it in Settings".into());
        }
        apply_token_env(command, &home)?;
        None
    } else {
        let lease = subscription_accounts::acquire_runtime_lease(app, id, harness)?;
        apply_subscription_env(command, harness, &home)?;
        command.env("BUZZ_ACP_AGENTS", "1");
        Some(lease)
    };
    command
        .env("BUZZ_ACP_SUBSCRIPTION_ACCOUNT_ID", id)
        .env(
            "BUZZ_ACP_SUBSCRIPTION_ACCOUNT_GENERATION",
            record.subscription_account_generation.to_string(),
        )
        .env("BUZZ_ACP_SUBSCRIPTION_KIND", &account.kind)
        .env("BUZZ_ACP_SUBSCRIPTION_HARNESS", harness)
        .env("BUZZ_ACP_SUBSCRIPTION_HOME", &home);
    Ok(Some(RuntimeBinding {
        account_id: id.into(),
        generation: record.subscription_account_generation,
        _lease: lease,
    }))
}

const AUTH_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR",
    "CLAUDE_CODE_API_KEY_HELPER_TTL_MS",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_ANTHROPIC_AWS",
    "CLAUDE_CODE_USE_ANTHROPIC_GOOGLE_CLOUD",
    "CLAUDE_CODE_USE_MANTLE",
    "ANTHROPIC_PROFILE",
    "CLAUDE_CODE_API_KEY_FILE_DESCRIPTOR",
    "CLAUDE_CODE_GATEWAY_TOKEN_FILE_DESCRIPTOR",
    "CLAUDE_CODE_HOST_AUTH_ENV_VAR",
    "CLAUDE_CODE_HOST_CREDS_FILE",
    "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST",
    "ANTHROPIC_UNIX_SOCKET",
    "ANTHROPIC_CUSTOM_HEADERS",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_ORG_ID",
    "OPENAI_ORGANIZATION",
    "OPENAI_PROJECT_ID",
    "CODEX_API_KEY",
    "CODEX_API_KEY_FILE_DESCRIPTOR",
];
fn command_env(command: &Command, key: &str) -> Option<std::ffi::OsString> {
    command
        .get_envs()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.map(|v| v.to_owned()))
        .unwrap_or_else(|| std::env::var_os(key))
}
fn apply_subscription_env(command: &mut Command, harness: &str, home: &Path) -> Result<(), String> {
    match harness {
        "claude" => {
            // Configuration remains in the existing per-agent context. Reject
            // config-driven authentication instead of letting it outrank OAuth.
            let config_home = command_env(command, "CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .or_else(|| dirs::home_dir().map(|p| p.join(".claude")));
            if let Some(config_home) = config_home {
                if !config_home.is_absolute() {
                    return Err("Claude configuration home must be absolute for subscription account selection".into());
                }
                validate_claude_config(&config_home, command.get_current_dir())?;
                command
                    .env("CLAUDE_CONFIG_DIR", &config_home)
                    .env("BUZZ_ACP_SUBSCRIPTION_CONFIG_HOME", &config_home);
            }
            command.env("CLAUDE_SECURESTORAGE_CONFIG_DIR", home);
        }
        "codex" => {
            let source = command_env(command, "CODEX_HOME")
                .map(PathBuf::from)
                .or_else(|| dirs::home_dir().map(|p| p.join(".codex")))
                .ok_or("Cannot resolve Codex configuration home")?;
            if !source.is_absolute() {
                return Err(
                    "Codex configuration home must be absolute for subscription account selection"
                        .into(),
                );
            }
            validate_codex_project_config(command.get_current_dir())?;
            compose_codex_config(&source, home, command)?;
            command.env("CODEX_HOME", home);
        }
        _ => return Err("Unsupported subscription harness".into()),
    }
    for name in AUTH_ENV {
        command.env_remove(name);
    }
    Ok(())
}
fn apply_token_env(command: &mut Command, home: &Path) -> Result<(), String> {
    apply_subscription_env(command, "claude", home)?;
    command.env_remove("CLAUDE_SECURESTORAGE_CONFIG_DIR");
    let token = subscription_accounts::read_token(home)?;
    command.env("CLAUDE_CODE_OAUTH_TOKEN", token.as_str());
    Ok(())
}
fn validate_claude_config(home: &Path, cwd: Option<&Path>) -> Result<(), String> {
    for path in [
        home.join("settings.json"),
        home.join("settings.local.json"),
        PathBuf::from("/Library/Application Support/ClaudeCode/managed-settings.json"),
        PathBuf::from("/etc/claude-code/managed-settings.json"),
    ] {
        let Some(raw) = read_optional_config(&path)? else {
            continue;
        };
        let value: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|_| "Claude settings cannot be validated for subscription authentication")?;
        reject_auth_config(&value)?;
    }
    // Project settings have higher precedence than the account home too.
    if let Some(cwd) = cwd {
        for parent in cwd.ancestors() {
            for name in ["settings.json", "settings.local.json"] {
                let path = parent.join(".claude").join(name);
                if let Some(raw) = read_optional_config(&path)? {
                    let value = serde_json::from_str(&raw)
                        .map_err(|_| "Project Claude settings cannot be validated")?;
                    reject_auth_config(&value)?;
                }
            }
        }
    }
    Ok(())
}
fn read_optional_config(path: &Path) -> Result<Option<String>, String> {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
        Ok(m) => {
            if !m.is_file() || m.len() > 1024 * 1024 {
                return Err("Harness configuration is not a bounded regular file".into());
            }
            std::fs::read_to_string(path)
                .map(Some)
                .map_err(|e| e.to_string())
        }
    }
}
fn reject_auth_config(value: &serde_json::Value) -> Result<(), String> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                let lower = key.to_ascii_lowercase();
                if AUTH_ENV.iter().any(|s| s.eq_ignore_ascii_case(key))
                    || [
                        "apikeyhelper",
                        "api_key",
                        "api_key_helper",
                        "bearer_token",
                        "experimental_bearer_token",
                        "env_key",
                        "base_url",
                        "model_providers",
                        "forced_chatgpt_workspace_id",
                    ]
                    .contains(&lower.as_str())
                {
                    return Err("Harness configuration contains competing authentication; remove it before selecting a subscription account".into());
                }
                reject_auth_config(value)?;
            }
        }
        serde_json::Value::Array(v) => {
            for value in v {
                reject_auth_config(value)?
            }
        }
        _ => {}
    }
    Ok(())
}
fn validate_codex_project_config(cwd: Option<&Path>) -> Result<(), String> {
    if let Some(cwd) = cwd {
        for parent in cwd.ancestors() {
            if let Some(raw) = read_optional_config(&parent.join(".codex/config.toml"))? {
                let config: toml::Table = raw
                    .parse()
                    .map_err(|_| "Project Codex configuration cannot be validated")?;
                let value = serde_json::to_value(config).map_err(|e| e.to_string())?;
                reject_auth_config(&value)?;
                if value
                    .get("model_provider")
                    .and_then(|v| v.as_str())
                    .is_some_and(|p| p != "openai")
                {
                    return Err(
                        "Project Codex provider conflicts with subscription authentication".into(),
                    );
                }
            }
        }
    }
    for path in [
        "/etc/codex/config.toml",
        "/Library/Application Support/Codex/managed_config.toml",
    ] {
        if let Some(raw) = read_optional_config(Path::new(path))? {
            let config: toml::Table = raw
                .parse()
                .map_err(|_| "Managed Codex config cannot be validated")?;
            reject_auth_config(&serde_json::to_value(config).map_err(|e| e.to_string())?)?;
        }
    }
    Ok(())
}
fn normalize_config_paths(table: &mut toml::Table, source: &Path) -> Result<(), String> {
    normalize_config_table_paths(table, source, &[])
}

fn normalize_config_table_paths(
    table: &mut toml::Table,
    source: &Path,
    scope: &[&str],
) -> Result<(), String> {
    for (key, value) in table {
        let mut child_scope = scope.to_vec();
        child_scope.push(key.as_str());
        match value {
            toml::Value::Table(child) => normalize_config_table_paths(child, source, &child_scope)?,
            // MCP cwd is passed to process launch, not resolved against CODEX_HOME.
            // Account selection preserves the launch directory, so preserve this
            // value too. Rebasing it onto either config home would change behavior.
            toml::Value::String(_)
                if key == "cwd" && scope.len() == 2 && scope[0] == "mcp_servers" => {}
            toml::Value::String(path)
                if key.ends_with("_file")
                    || key.ends_with("_dir")
                    || key.ends_with("_path")
                    || key == "model_catalog_json" =>
            {
                if path.starts_with('~') {
                    return Err("Codex configuration path uses shell expansion; use an absolute path before selecting an account".into());
                }
                if !path.is_empty() && !Path::new(path).is_absolute() {
                    *path = source.join(&*path).to_string_lossy().into_owned();
                }
            }
            toml::Value::String(path)
                if (key == "path" || key == "cwd")
                    && !path.is_empty()
                    && !Path::new(path).is_absolute() =>
            {
                return Err("Codex configuration contains an unsupported relative path; use an absolute path before selecting an account".into());
            }
            toml::Value::Array(values) => {
                for value in values {
                    if let toml::Value::Table(child) = value {
                        normalize_config_table_paths(child, source, &child_scope)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn compose_codex_config(
    source: &Path,
    destination: &Path,
    command: &mut Command,
) -> Result<(), String> {
    let raw = read_optional_config(&source.join("config.toml"))?.unwrap_or_default();
    let mut config: toml::Table = raw
        .parse()
        .map_err(|_| "Codex configuration cannot be parsed")?;
    normalize_config_paths(&mut config, source)?;
    reject_auth_config(&serde_json::to_value(&config).map_err(|e| e.to_string())?)?;
    if config
        .get("model_provider")
        .and_then(|v| v.as_str())
        .is_some_and(|p| p != "openai")
    {
        return Err("Codex model_provider conflicts with subscription authentication".into());
    }
    // Preserve supported non-secret configuration assets by reference, never
    // copying credentials, history, token caches, or arbitrary home contents.
    for name in [
        "AGENTS.md",
        "AGENTS.override.md",
        "skills",
        "rules",
        "plugins",
    ] {
        let src = source.join(name);
        let dst = destination.join(name);
        if src.exists() {
            #[cfg(unix)]
            {
                if std::fs::symlink_metadata(&dst).is_ok() {
                    std::fs::remove_file(&dst)
                        .map_err(|_| "Owned Codex configuration link cannot be replaced")?;
                }
                std::os::unix::fs::symlink(&src, &dst).map_err(|e| e.to_string())?;
            }
            #[cfg(not(unix))]
            {
                return Err("This platform cannot preserve Codex configuration assets yet".into());
            }
        } else if std::fs::symlink_metadata(&dst).is_ok() {
            std::fs::remove_file(&dst).map_err(|e| e.to_string())?;
        }
    }
    config.insert(
        "cli_auth_credentials_store".into(),
        toml::Value::String("file".into()),
    );
    config.insert(
        "forced_login_method".into(),
        toml::Value::String("chatgpt".into()),
    );
    let raw = toml::to_string(&config).map_err(|e| e.to_string())?;
    let config_path = destination.join("config.toml");
    if std::fs::symlink_metadata(&config_path)
        .is_ok_and(|m| m.file_type().is_symlink() || !m.is_file())
    {
        return Err("Owned Codex config must be a regular file".into());
    }
    super::atomic_write_json_restricted(&config_path, raw.as_bytes())?;
    if let Some(extra) = command_env(command, "CODEX_CONFIG") {
        let mut value: serde_json::Value = serde_json::from_str(&extra.to_string_lossy())
            .map_err(|_| "CODEX_CONFIG cannot be validated")?;
        reject_auth_config(&value)?;
        let map = value
            .as_object_mut()
            .ok_or("CODEX_CONFIG must be an object")?;
        if map
            .get("model_provider")
            .and_then(|v| v.as_str())
            .is_some_and(|p| p != "openai")
        {
            return Err(
                "CODEX_CONFIG model_provider conflicts with subscription authentication".into(),
            );
        }
        map.insert("cli_auth_credentials_store".into(), "file".into());
        map.insert("forced_login_method".into(), "chatgpt".into());
        command.env("CODEX_CONFIG", value.to_string());
    }
    Ok(())
}

/// Every new broker gets a private generation-specific admission barrier.
pub(crate) fn configure_control(
    app: &AppHandle,
    key: &super::ManagedAgentRuntimeKey,
    nonce: &str,
    command: &mut Command,
) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("agents")
        .join("subscription-runtime");
    private_runtime_dir(&base)?;
    let root = base.join(key.runtime_id());
    private_runtime_dir(&root)?;
    let dir = root.join(nonce);
    private_runtime_dir(&dir)?;
    command
        .env("BUZZ_ACP_ACCOUNT_CONTROL_DIR", &dir)
        .env("BUZZ_ACP_AUTH_RECOVERY_DIR", root.join("auth-recovery"));
    Ok(dir)
}
fn private_runtime_dir(path: &Path) -> Result<(), String> {
    if !path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(path)
                .map_err(|e| e.to_string())?;
        }
        #[cfg(not(unix))]
        {
            std::fs::create_dir(path).map_err(|e| e.to_string())?;
        }
    }
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("Subscription runtime path must be an owned directory".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err("Subscription runtime directory must be private".into());
        }
    }
    Ok(())
}

fn read_control_response(path: &Path) -> Result<Option<String>, String> {
    use std::io::Read;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    // Validate the object we actually opened, rather than checking a path and
    // subsequently following a replaced symlink into unrelated local data.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Invalid subscription control response path".into()),
    };
    let metadata = file
        .metadata()
        .map_err(|_| "Invalid subscription control response")?;
    if !metadata.is_file() || metadata.len() > 2048 {
        return Err("Subscription control response exceeds its bounded file contract".into());
    }
    let mut bytes = Vec::new();
    file.take(2049)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read subscription control response")?;
    if bytes.len() > 2048 {
        return Err("Subscription control response exceeds its bounded file contract".into());
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "Invalid subscription control response encoding".into())
}

fn control(dir: &Path, nonce: &str, token: &str, action: &str) -> Result<(), String> {
    if std::fs::symlink_metadata(dir.join("request.json"))
        .is_ok_and(|m| !m.is_file() || m.file_type().is_symlink())
    {
        return Err("Invalid subscription control request path".into());
    }
    super::atomic_write_json_restricted(
        &dir.join("request.json"),
        serde_json::json!({"nonce":nonce,"token":token,"action":action})
            .to_string()
            .as_bytes(),
    )?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Some(raw) = read_control_response(&dir.join("response.json"))? {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
                if value["nonce"] == nonce && value["token"] == token {
                    match value["status"].as_str() {
                        Some("quiesced") if action == "quiesce" => return Ok(()),
                        Some("released") if action == "release" => return Ok(()),
                        Some("resumed") if action == "resume_auth" => return Ok(()),
                        Some("resume_failed") => {
                            return Err("Retained work could not be resumed; see agent error".into())
                        }
                        Some("busy") => {
                            return Err(
                                "Agent is busy; finish the active turn before changing accounts"
                                    .into(),
                            )
                        }
                        _ => {}
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Err(
        "Agent did not confirm an idle account-switch barrier; stop it before changing accounts"
            .into(),
    )
}

/// A saved selection and a successful spawn are separate facts.
#[derive(Serialize)]
pub struct SubscriptionSwitchResult {
    pub account_id: Option<String>,
    pub generation: u64,
    pub restarted_count: u32,
    pub failed_restart_count: u32,
    pub errors: Vec<String>,
}

/// Atomically fence admissions, stop idle children, save, then restart with a new generation.
#[tauri::command]
pub fn switch_agent_subscription_account(
    pubkey: String,
    account_id: Option<String>,
    app: AppHandle,
) -> Result<SubscriptionSwitchResult, String> {
    switch_account(pubkey, account_id, app, None)
}

/// Renewal may only restart bindings that still select this mint's account.
pub(crate) fn restart_agent_token_account(
    pubkey: String,
    id: String,
    minted: i64,
    app: AppHandle,
) -> Result<SubscriptionSwitchResult, String> {
    switch_account(pubkey, Some(id.clone()), app, Some((id, minted)))
}

fn renewal_still_selected(
    selected: Option<&str>,
    expected: &str,
    current_mint: Option<i64>,
    expected_mint: i64,
) -> bool {
    selected == Some(expected) && current_mint == Some(expected_mint)
}

fn switch_account(
    pubkey: String,
    account_id: Option<String>,
    app: AppHandle,
    expected: Option<(String, i64)>,
) -> Result<SubscriptionSwitchResult, String> {
    let state = app.state::<AppState>();
    super::require_fleet_host(&state)?;
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|e| e.to_string())?;
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let mut records = super::load_managed_agents(&app)?;
    let record = super::find_managed_agent_mut(&mut records, &pubkey)?;
    if record.backend != super::BackendKind::Local {
        return Err("Subscription accounts require a local agent".into());
    }
    let personas = super::load_personas(&app)?;
    let command = super::record_agent_command(record, &personas);
    let harness = account_harness(super::known_acp_runtime(&command).map(|r| r.id));
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    let account_guard = subscription_accounts::lifecycle_lock()
        .lock()
        .map_err(|e| e.to_string())?;
    if let Some((id, minted)) = expected.as_ref() {
        if !renewal_still_selected(
            record.subscription_account_id.as_deref(),
            id,
            subscription_accounts::load_account(&app, id)?.minted_at,
            *minted,
        ) {
            return Ok(SubscriptionSwitchResult {
                account_id: record.subscription_account_id.clone(),
                generation: record.subscription_account_generation,
                restarted_count: 0,
                failed_restart_count: 0,
                errors: Vec::new(),
            });
        }
    }
    if let Some(id) = account_id.as_deref() {
        if !subscription_accounts::accounts_enabled(&app)?
            && record.subscription_account_id.as_deref() != Some(id)
        {
            return Err("Enable Subscription Accounts to select a new account".into());
        }
        let account = subscription_accounts::load_account(&app, id)?;
        if harness != Some(account.harness.as_str()) {
            return Err("Account and agent harness do not match".into());
        }
        if account.state != "ready"
            || account
                .nickname
                .as_deref()
                .is_none_or(|n| n.trim().is_empty())
        {
            return Err("Account is not ready; reconnect it in Settings".into());
        }
        subscription_accounts::account_home(&app, id)?;
    }
    let keys = super::managed_agent_runtime_keys(&runtimes, &pubkey);
    let owned_lease = keys
        .first()
        .and_then(|key| runtimes.get(key))
        .and_then(|runtime| {
            runtime.subscription_binding.as_ref().and_then(|binding| {
                binding
                    ._lease
                    .as_ref()
                    .map(|lease| (lease, runtime.child.id()))
            })
        });
    let admission = subscription_accounts::prepare_switch(
        &app,
        account_id.as_deref(),
        keys.len(),
        owned_lease,
    )?;
    // Admission and effects share one execution seam: a busy destination cannot
    // quiesce a working runtime or persist a different account selection.
    admission.execute(|| {
        let token = uuid::Uuid::new_v4().simple().to_string();
        let mut held: Vec<(PathBuf, String)> = Vec::new();
        for key in &keys {
            if let Some(runtime) = runtimes.get(key) {
                let outcome = runtime
                    .account_control_dir
                    .as_ref()
                    .ok_or_else(|| {
                        "Running agent predates safe account switching; stop it first".to_string()
                    })
                    .and_then(|dir| {
                        held.push((dir.clone(), runtime.start_nonce.clone()));
                        control(dir, &runtime.start_nonce, &token, "quiesce")
                    });
                if let Err(e) = outcome {
                    for (dir, nonce) in &held {
                        let _ = control(dir, nonce, &token, "release");
                    }
                    return Err(e);
                }
            }
        }
        // Teardown precedes persist, so no old-account runtime survives a saved change.
        for key in &keys {
            if let Some(mut runtime) = runtimes.remove(key) {
                if let Err(e) = super::terminate_process(runtime.child.id())
                    .and_then(|()| runtime.child.wait().map(|_| ()).map_err(|e| e.to_string()))
                {
                    runtimes.insert(key.clone(), runtime);
                    for (dir, nonce) in &held {
                        let _ = control(dir, nonce, &token, "release");
                    }
                    return Err(e);
                }
                super::remove_agent_runtime_receipt(&app, key);
                state.clear_agent_session_cache(key);
            }
        }
        let record = super::find_managed_agent_mut(&mut records, &pubkey)?;
        record.subscription_account_id = account_id.clone();
        record.subscription_account_generation = record
            .subscription_account_generation
            .checked_add(1)
            .ok_or("Subscription binding generation exhausted")?;
        // Explicit ambient selection must override inherited legacy Claude accounts.
        record.claude_account_id = Some(super::CREDENTIALS_FILE_ACCOUNT_ID.into());
        record.updated_at = crate::util::now_iso();
        record.last_error = None;
        let generation = record.subscription_account_generation;
        let snapshot = record.clone();
        super::save_managed_agents(&app, &records)?;
        drop(account_guard);
        let owner = state
            .keys
            .lock()
            .ok()
            .map(|keys| keys.public_key().to_hex());
        let mut result = SubscriptionSwitchResult {
            account_id,
            generation,
            restarted_count: 0,
            failed_restart_count: 0,
            errors: Vec::new(),
        };
        for key in keys {
            match super::spawn_agent_child(&app, &snapshot, &key.relay_url, true, owner.as_deref())
            {
                Ok(mut process) => {
                    let receipt = super::ManagedAgentRuntimeReceipt {
                        subscription_account_id: process
                            .subscription_binding
                            .as_ref()
                            .map(|b| b.account_id.clone()),
                        subscription_account_generation: process
                            .subscription_binding
                            .as_ref()
                            .map_or(0, |b| b.generation),
                        key: key.clone(),
                        pid: process.child.id(),
                        desktop_instance_id: super::current_instance_id(&app),
                        started_at: crate::util::now_iso(),
                    };
                    if let Err(e) = super::write_agent_runtime_receipt(&app, &receipt) {
                        let _ = super::terminate_process(process.child.id());
                        let _ = process.child.wait();
                        result.failed_restart_count += 1;
                        result.errors.push(e);
                    } else {
                        runtimes.insert(key, super::ManagedAgentPairRuntime::starting(process));
                        result.restarted_count += 1;
                    }
                }
                Err(e) => {
                    result.failed_restart_count += 1;
                    result.errors.push(e);
                }
            }
        }
        if !result.errors.is_empty() {
            let record = super::find_managed_agent_mut(&mut records, &pubkey)?;
            record.last_error = Some(result.errors.join("; "));
            super::save_managed_agents(&app, &records)?;
        }
        Ok(result)
    })
}

pub(crate) fn ready_binding(
    runtime: Option<&super::ManagedAgentPairRuntime>,
) -> Option<&RuntimeBinding> {
    runtime
        .filter(|r| r.lifecycle == ManagedAgentRuntimeLifecycle::Ready)
        .and_then(|r| r.subscription_binding.as_ref())
}

/// Explicitly resume retained work after reconnect; never an automatic login side effect.
#[tauri::command]
pub fn resume_agent_subscription_work(pubkey: String, app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    super::require_fleet_host(&state)?;
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|e| e.to_string())?;
    let runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    let keys = super::managed_agent_runtime_keys(&runtimes, &pubkey);
    if keys.is_empty() {
        return Err("Start the agent before resuming retained work".into());
    }
    for key in keys {
        if let Some(runtime) = runtimes.get(&key) {
            let dir = runtime
                .account_control_dir
                .as_ref()
                .ok_or("Agent does not support authentication recovery; restart it")?;
            control(
                dir,
                &runtime.start_nonce,
                &uuid::Uuid::new_v4().simple().to_string(),
                "resume_auth",
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "subscription_binding_tests.rs"]
mod tests;
