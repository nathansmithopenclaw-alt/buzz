//! First-class Claude OAuth account registry.
//!
//! Metadata is Desktop-owned JSON. OAuth bearer values live only in the
//! existing `SecretStore` blob and are hydrated at spawn from a structured
//! account id.

use std::{
    collections::BTreeMap,
    process::Command,
    sync::{Mutex, OnceLock},
};

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::{
    app_state::{keyring_service, AppState},
    managed_agents::{
        atomic_write_json, load_global_agent_config, load_managed_agents, load_personas,
        managed_agent_runtime_keys, save_global_agent_config, save_managed_agents, save_personas,
        AgentDefinition, GlobalAgentConfig, ManagedAgentPairRuntime, ManagedAgentRecord,
    },
    secret_store::SecretStore,
    util::now_iso,
};

const REGISTRY_FILE: &str = "claude-accounts.json";
const MIGRATION_MARKER: &str = "claude-accounts-migrated-v1";
const TOKEN_LENGTH: usize = 108;
pub const CREDENTIALS_FILE_ACCOUNT_ID: &str = "__credentials_file__";
const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";

fn registry_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClaudeAccountMetadata {
    pub id: String,
    pub label: String,
    pub fingerprint: String,
    pub length: usize,
    pub last_verified: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeUsageWindow {
    pub utilization: Option<f64>,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeModelUsage {
    pub model: String,
    pub utilization: Option<f64>,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeAccountView {
    #[serde(flatten)]
    pub metadata: ClaudeAccountMetadata,
    pub plan_type: Option<String>,
    pub five_hour: Option<ClaudeUsageWindow>,
    pub weekly: Option<ClaudeUsageWindow>,
    pub model_scoped: Vec<ClaudeModelUsage>,
    /// `available`, `scope_insufficient`, or `unavailable`.
    pub usage_status: String,
    pub usage_http_status: Option<u16>,
    pub usage_error: Option<String>,
    pub usage_checked_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeAccountSwitchResult {
    pub account_id: Option<String>,
    pub fingerprint: Option<String>,
    pub restarted_count: u32,
    pub failed_restart_count: u32,
    pub verified_runtime_count: u32,
    pub running_pids: Vec<u32>,
}

fn append_switch_log_markers(app: &AppHandle, pubkey: &str, message: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;
    for key in managed_agent_runtime_keys(&runtimes, pubkey) {
        if let Some(runtime) = runtimes.get(&key) {
            crate::managed_agents::append_log_marker(
                &runtime.log_path,
                &format!("[{}] {message}", now_iso()),
            )?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn note_claude_account_switch_queued(pubkey: String, app: AppHandle) -> Result<(), String> {
    append_switch_log_markers(
        &app,
        &pubkey,
        "Claude account switch queued; waiting for the active turn to complete",
    )
}

fn registry_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("agents");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
    Ok(dir.join(REGISTRY_FILE))
}

fn migration_marker_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(registry_path(app)?.with_file_name(MIGRATION_MARKER))
}

fn load_registry(app: &AppHandle) -> Result<Vec<ClaudeAccountMetadata>, String> {
    let path = registry_path(app)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes =
        std::fs::read(&path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("failed to parse {}: {e}", path.display()))
}

fn save_registry(app: &AppHandle, accounts: &[ClaudeAccountMetadata]) -> Result<(), String> {
    let payload = serde_json::to_vec_pretty(accounts).map_err(|e| e.to_string())?;
    atomic_write_json(&registry_path(app)?, &payload)
}

fn token_key(id: &str) -> String {
    format!("claude-account:{id}")
}

fn secret_store() -> &'static SecretStore {
    SecretStore::shared(keyring_service())
}

pub fn normalize_claude_token(raw: &str) -> Result<String, String> {
    let token: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    if token.chars().count() != TOKEN_LENGTH {
        return Err(format!(
            "Claude OAuth token must be {TOKEN_LENGTH} characters after whitespace is removed (got {})",
            token.chars().count()
        ));
    }
    if !token
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("Claude OAuth token may contain only letters, numbers, `_`, and `-`".into());
    }
    Ok(token)
}

pub fn claude_token_fingerprint(token: &str) -> String {
    crate::runtime_identity::secret_fingerprint(token)
}

pub fn load_claude_account_token(id: &str) -> Result<String, String> {
    secret_store()
        .load(&token_key(id))?
        .ok_or_else(|| format!("Claude account `{id}` has no keychain credential"))
}

pub fn effective_claude_account<'a>(
    record: &'a ManagedAgentRecord,
    definitions: &'a [AgentDefinition],
    global: &'a crate::managed_agents::GlobalAgentConfig,
) -> (Option<&'a str>, &'static str) {
    if let Some(id) = record.claude_account_id.as_deref() {
        if id == CREDENTIALS_FILE_ACCOUNT_ID {
            return (None, "credentials_file");
        }
        return (Some(id), "instance");
    }
    let inherited = record
        .persona_id
        .as_deref()
        .and_then(|persona_id| {
            definitions
                .iter()
                .find(|definition| definition.id == persona_id)
        })
        .and_then(|definition| definition.claude_account_id.as_deref());
    match inherited {
        Some(CREDENTIALS_FILE_ACCOUNT_ID) => (None, "credentials_file"),
        Some(id) => (Some(id), "definition"),
        None => match global.claude_account_id.as_deref() {
            Some(CREDENTIALS_FILE_ACCOUNT_ID) => (None, "credentials_file"),
            Some(id) => (Some(id), "global"),
            None => (None, "credentials_file"),
        },
    }
}

pub struct ClaudeAccountRuntimeSummary {
    pub account_id: Option<String>,
    pub source: String,
    pub shadowed: bool,
}

pub fn runtime_claude_account_summary(
    record: &ManagedAgentRecord,
    definitions: &[AgentDefinition],
    global: &GlobalAgentConfig,
    runtime: Option<&ManagedAgentPairRuntime>,
) -> ClaudeAccountRuntimeSummary {
    let (configured_id, configured_source) = effective_claude_account(record, definitions, global);
    let account_id = runtime
        .map(|process| process.claude_account_id.as_deref())
        .unwrap_or(configured_id);
    let source = runtime
        .map(|process| process.claude_account_source.as_str())
        .unwrap_or(configured_source);
    let inherited_id = record
        .persona_id
        .as_deref()
        .and_then(|persona_id| {
            definitions
                .iter()
                .find(|definition| definition.id == persona_id)
        })
        .and_then(|definition| definition.claude_account_id.as_deref())
        .or(global.claude_account_id.as_deref());
    ClaudeAccountRuntimeSummary {
        account_id: account_id.map(str::to_string),
        source: source.to_string(),
        shadowed: record.claude_account_id.is_some()
            && inherited_id.is_some()
            && record.claude_account_id.as_deref() != inherited_id,
    }
}

pub fn configure_spawned_claude_account(
    command: &mut Command,
    is_claude_runtime: bool,
    record: &ManagedAgentRecord,
    definitions: &[AgentDefinition],
    global: &GlobalAgentConfig,
) -> Result<(Option<String>, Option<String>, String), String> {
    if !is_claude_runtime {
        return Ok((None, None, "credentials_file".into()));
    }
    command.env_remove("CLAUDE_CODE_OAUTH_TOKEN");
    let (account_id, source) = effective_claude_account(record, definitions, global);
    let account_id = account_id.map(str::to_string);
    let fingerprint = match account_id.as_deref() {
        Some(id) => {
            let token = load_claude_account_token(id)?;
            let fingerprint = claude_token_fingerprint(&token);
            command.env("CLAUDE_CODE_OAUTH_TOKEN", token);
            Some(fingerprint)
        }
        None => None,
    };
    Ok((account_id, fingerprint, source.into()))
}

fn usage_window(value: Option<&serde_json::Value>) -> Option<ClaudeUsageWindow> {
    let value = value?;
    Some(ClaudeUsageWindow {
        utilization: value.get("utilization").and_then(serde_json::Value::as_f64),
        resets_at: value
            .get("resets_at")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    })
}

async fn fetch_usage(
    client: &reqwest::Client,
    token: &str,
    metadata: ClaudeAccountMetadata,
) -> ClaudeAccountView {
    let mut response = None;
    for attempt in 0..3 {
        match client
            .get(USAGE_URL)
            .header("anthropic-beta", OAUTH_BETA)
            .bearer_auth(token)
            .send()
            .await
        {
            Ok(candidate) if candidate.status() == StatusCode::TOO_MANY_REQUESTS && attempt < 2 => {
                tokio::time::sleep(std::time::Duration::from_secs(1 << attempt)).await;
            }
            Ok(candidate) => {
                response = Some(candidate);
                break;
            }
            Err(error) => {
                return ClaudeAccountView {
                    metadata,
                    plan_type: None,
                    five_hour: None,
                    weekly: None,
                    model_scoped: Vec::new(),
                    usage_status: "unavailable".into(),
                    usage_http_status: None,
                    usage_error: Some(format!("usage request failed: {error}")),
                    usage_checked_at: None,
                }
            }
        }
    }
    let Some(response) = response else {
        return ClaudeAccountView {
            metadata,
            plan_type: None,
            five_hour: None,
            weekly: None,
            model_scoped: Vec::new(),
            usage_status: "stale".into(),
            usage_http_status: Some(429),
            usage_error: Some(
                "Usage endpoint is rate limited; showing the last successful check when available."
                    .into(),
            ),
            usage_checked_at: Some(now_iso()),
        };
    };
    let status = response.status();
    let body = response
        .json::<serde_json::Value>()
        .await
        .unwrap_or(serde_json::Value::Null);
    if status != StatusCode::OK {
        let message = body
            .pointer("/error/message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("usage data unavailable")
            .to_string();
        let (usage_status, message) =
            if status == StatusCode::FORBIDDEN && message.contains("user:profile") {
                (
                    "scope_insufficient",
                    "Usage meter needs a login-minted token; re-mint with interactive login.",
                )
            } else if status == StatusCode::TOO_MANY_REQUESTS {
                (
                "stale",
                "Usage endpoint is rate limited; showing the last successful check when available.",
            )
            } else {
                ("unavailable", message.as_str())
            };
        return ClaudeAccountView {
            metadata,
            plan_type: None,
            five_hour: None,
            weekly: None,
            model_scoped: Vec::new(),
            usage_status: usage_status.into(),
            usage_http_status: Some(status.as_u16()),
            usage_error: Some(message.to_string()),
            usage_checked_at: Some(now_iso()),
        };
    }
    let model_scoped = body
        .get("model_scoped")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(|entry| ClaudeModelUsage {
            model: entry
                .get("model")
                .or_else(|| entry.get("name"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Model")
                .to_string(),
            utilization: entry.get("utilization").and_then(serde_json::Value::as_f64),
            resets_at: entry
                .get("resets_at")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
        })
        .collect();
    ClaudeAccountView {
        metadata,
        plan_type: body
            .get("subscription_type")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        five_hour: usage_window(body.get("five_hour")),
        weekly: usage_window(body.get("seven_day")),
        model_scoped,
        usage_status: "available".into(),
        usage_http_status: Some(200),
        usage_error: None,
        usage_checked_at: Some(now_iso()),
    }
}

/// A minimal real inference call is the validity gate. Usage scope is
/// deliberately not part of validity: setup-token mints infer successfully
/// while lacking `user:profile`.
async fn probe_inference(client: &reqwest::Client, token: &str) -> Result<Option<u16>, String> {
    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("content-type", "application/json")
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", OAUTH_BETA)
        .bearer_auth(token)
        .json(&serde_json::json!({
            "model": "claude-haiku-4-5-20251001",
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "Reply with OK."}]
        }))
        .send()
        .await;
    let response = match response {
        Ok(response) => response,
        // Network/transient failures degrade to saved-but-unverified. A
        // definitive OAuth rejection below is the only save blocker.
        Err(_) => return Ok(None),
    };
    let status = response.status();
    if status == StatusCode::UNAUTHORIZED {
        let body = response
            .json::<serde_json::Value>()
            .await
            .unwrap_or(serde_json::Value::Null);
        let detail = body
            .pointer("/error/message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("token rejected");
        return Err(format!("Claude rejected this token (401): {detail}"));
    }
    Ok(Some(status.as_u16()))
}

#[tauri::command]
pub async fn list_claude_accounts(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ClaudeAccountView>, String> {
    let metadata = load_registry(&app)?;
    Ok(
        futures_util::future::join_all(metadata.into_iter().map(|account| async {
            match load_claude_account_token(&account.id) {
                Ok(token) => fetch_usage(&state.http_client, &token, account).await,
                Err(error) => ClaudeAccountView {
                    metadata: account,
                    plan_type: None,
                    five_hour: None,
                    weekly: None,
                    model_scoped: Vec::new(),
                    usage_status: "unavailable".into(),
                    usage_http_status: None,
                    usage_error: Some(error),
                    usage_checked_at: None,
                },
            }
        }))
        .await,
    )
}

#[tauri::command]
pub async fn add_claude_account(
    label: String,
    token: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ClaudeAccountView, String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("Account label is required".into());
    }
    let token = normalize_claude_token(&token)?;
    let mut metadata = ClaudeAccountMetadata {
        id: uuid::Uuid::new_v4().to_string(),
        label: label.to_string(),
        fingerprint: claude_token_fingerprint(&token),
        length: token.len(),
        last_verified: None,
    };
    let inference_status = probe_inference(&state.http_client, &token).await?;
    let probe = fetch_usage(&state.http_client, &token, metadata.clone()).await;
    if inference_status == Some(200) {
        metadata.last_verified = Some(now_iso());
    }
    let _guard = registry_lock().lock().map_err(|e| e.to_string())?;
    let mut accounts = load_registry(&app)?;
    if accounts
        .iter()
        .any(|account| account.fingerprint == metadata.fingerprint)
    {
        return Err("This Claude account token is already saved".into());
    }
    secret_store().store(&token_key(&metadata.id), &token)?;
    accounts.push(metadata.clone());
    if let Err(error) = save_registry(&app, &accounts) {
        let _ = secret_store().delete(&token_key(&metadata.id));
        return Err(error);
    }
    Ok(ClaudeAccountView { metadata, ..probe })
}

#[tauri::command]
pub fn delete_claude_account(id: String, app: AppHandle) -> Result<(), String> {
    let _guard = registry_lock().lock().map_err(|e| e.to_string())?;
    let records = load_managed_agents(&app)?;
    let personas = load_personas(&app)?;
    let global = load_global_agent_config(&app)?;
    if records
        .iter()
        .any(|record| record.claude_account_id.as_deref() == Some(&id))
        || personas
            .iter()
            .any(|definition| definition.claude_account_id.as_deref() == Some(&id))
        || global.claude_account_id.as_deref() == Some(&id)
    {
        return Err(
            "This account is assigned to an agent; switch that agent before deleting it".into(),
        );
    }
    let mut accounts = load_registry(&app)?;
    let before = accounts.len();
    accounts.retain(|account| account.id != id);
    if accounts.len() == before {
        return Err("Claude account not found".into());
    }
    save_registry(&app, &accounts)?;
    secret_store().delete(&token_key(&id))
}

#[tauri::command]
pub async fn switch_agent_claude_account(
    pubkey: String,
    account_id: Option<String>,
    deferred_after_turn: Option<bool>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ClaudeAccountSwitchResult, String> {
    let credentials_file_selected = account_id.as_deref() == Some(CREDENTIALS_FILE_ACCOUNT_ID);
    let effective_account_id = if credentials_file_selected {
        None
    } else {
        account_id.clone()
    };
    let requested_fingerprint = if let Some(id) = effective_account_id.as_deref() {
        let account = load_registry(&app)?
            .into_iter()
            .find(|account| account.id == id)
            .ok_or_else(|| "Claude account not found".to_string())?;
        let token = load_claude_account_token(id)?;
        // Re-check at the moment of use. Claude Code silently falls back to
        // ~/.claude/.credentials.json for a rejected env token, so trusting a
        // historical verification would make the UI report the wrong account.
        probe_inference(&state.http_client, &token).await?;
        Some(account.fingerprint)
    } else {
        None
    };
    let relay_urls = {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let mut records = load_managed_agents(&app)?;
        let record = records
            .iter_mut()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        record.claude_account_id = account_id.clone();
        record
            .env_vars
            .retain(|key, _| !key.eq_ignore_ascii_case("CLAUDE_CODE_OAUTH_TOKEN"));
        record.updated_at = now_iso();
        let runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| e.to_string())?;
        let urls = managed_agent_runtime_keys(&runtimes, &pubkey)
            .into_iter()
            .map(|key| key.relay_url)
            .collect::<Vec<_>>();
        drop(runtimes);
        save_managed_agents(&app, &records)?;
        urls
    };
    if deferred_after_turn.unwrap_or(false) {
        append_switch_log_markers(
            &app,
            &pubkey,
            "Active turn completed; applying queued Claude account switch and restarting",
        )?;
    }
    let mut restarted_count = 0;
    let mut failed_restart_count = 0;
    for relay_url in relay_urls {
        match super::restart_managed_agent_runtime(pubkey.clone(), relay_url, app.clone()) {
            Ok(_) => restarted_count += 1,
            Err(error) => {
                failed_restart_count += 1;
                eprintln!(
                    "buzz-desktop: Claude account switch failed to restart {pubkey}: {error}"
                );
            }
        }
    }
    let (verified_runtime_count, running_pids) = {
        let runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|error| error.to_string())?;
        let matching = managed_agent_runtime_keys(&runtimes, &pubkey)
            .into_iter()
            .filter_map(|key| runtimes.get(&key))
            .filter(|runtime| {
                runtime.claude_account_id.as_deref() == effective_account_id.as_deref()
                    && runtime.claude_token_fingerprint.as_deref()
                        == requested_fingerprint.as_deref()
            })
            .collect::<Vec<_>>();
        (
            matching.len() as u32,
            matching.iter().map(|runtime| runtime.child.id()).collect(),
        )
    };
    Ok(ClaudeAccountSwitchResult {
        account_id: effective_account_id,
        fingerprint: requested_fingerprint,
        restarted_count,
        failed_restart_count,
        verified_runtime_count,
        running_pids,
    })
}

fn take_legacy_tokens(env: &mut BTreeMap<String, String>) -> Vec<String> {
    let keys = env
        .keys()
        .filter(|key| {
            let upper = key.to_ascii_uppercase();
            upper == "CLAUDE_CODE_OAUTH_TOKEN" || upper.ends_with("_CLAUDE_CODE_OAUTH_TOKEN")
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut tokens = Vec::new();
    for key in keys {
        let Some(raw) = env.remove(&key) else {
            continue;
        };
        match normalize_claude_token(&raw) {
            Ok(token) => {
                // Prefer the canonical key when multiple legacy aliases were
                // present; all distinct credentials are still imported.
                let canonical = key.eq_ignore_ascii_case("CLAUDE_CODE_OAUTH_TOKEN");
                tokens.push((canonical, token));
            }
            Err(error) => eprintln!(
                "buzz-desktop: removed invalid plaintext Claude credential `{key}` during migration: {error}"
            ),
        }
    }
    tokens.sort_by_key(|(canonical, _)| !canonical);
    tokens.into_iter().map(|(_, token)| token).collect()
}

fn import_migrated_token(
    accounts: &mut Vec<ClaudeAccountMetadata>,
    label: String,
    token: &str,
) -> Result<String, String> {
    let token_fingerprint = claude_token_fingerprint(token);
    if let Some(existing) = accounts
        .iter()
        .find(|account| account.fingerprint == token_fingerprint)
    {
        return Ok(existing.id.clone());
    }
    let id = uuid::Uuid::new_v4().to_string();
    secret_store().store(&token_key(&id), token)?;
    accounts.push(ClaudeAccountMetadata {
        id: id.clone(),
        label,
        fingerprint: token_fingerprint,
        length: token.len(),
        last_verified: None,
    });
    Ok(id)
}

/// One-time startup migration for legacy plaintext Claude token env vars.
pub fn migrate_plaintext_claude_tokens(app: &AppHandle) -> Result<(), String> {
    let marker = migration_marker_path(app)?;
    if marker.exists() {
        return Ok(());
    }
    let _guard = registry_lock().lock().map_err(|e| e.to_string())?;
    let mut accounts = load_registry(app)?;
    let mut records = load_managed_agents(app)?;
    let mut definitions = load_personas(app)?;
    let mut records_changed = false;
    let mut definitions_changed = false;
    for record in &mut records {
        let tokens = take_legacy_tokens(&mut record.env_vars);
        let mut account_ids = Vec::new();
        for token in &tokens {
            account_ids.push(import_migrated_token(
                &mut accounts,
                format!("{} (migrated)", record.name),
                token,
            )?);
        }
        if let Some(account_id) = account_ids.into_iter().next() {
            record.claude_account_id = Some(account_id);
        }
        if !tokens.is_empty() {
            records_changed = true;
        }
    }
    for definition in &mut definitions {
        let tokens = take_legacy_tokens(&mut definition.env_vars);
        let mut account_ids = Vec::new();
        for token in &tokens {
            account_ids.push(import_migrated_token(
                &mut accounts,
                format!("{} (migrated)", definition.display_name),
                token,
            )?);
        }
        if let Some(account_id) = account_ids.into_iter().next() {
            definition.claude_account_id = Some(account_id);
        }
        if !tokens.is_empty() {
            definitions_changed = true;
        }
    }
    let mut global = load_global_agent_config(app)?;
    let global_tokens = take_legacy_tokens(&mut global.env_vars);
    for token in &global_tokens {
        let account_id = import_migrated_token(
            &mut accounts,
            "Global Claude account (migrated)".into(),
            token,
        )?;
        if global.claude_account_id.is_none() {
            global.claude_account_id = Some(account_id);
        }
    }
    let global_changed = !global_tokens.is_empty();
    // Import the interactive-login credential as a first-class account too.
    // This leaves the original credentials file untouched; it only gives the
    // account switcher a keychain-backed copy and enables the usage meter's
    // `user:profile` scope when present.
    if let Some(credentials_path) =
        dirs::home_dir().map(|home| home.join(".claude/.credentials.json"))
    {
        if let Ok(bytes) = std::fs::read(&credentials_path) {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if let Some(raw) = value
                    .pointer("/claudeAiOauth/accessToken")
                    .and_then(serde_json::Value::as_str)
                {
                    if let Ok(token) = normalize_claude_token(raw) {
                        import_migrated_token(
                            &mut accounts,
                            "Claude interactive login (migrated)".into(),
                            &token,
                        )?;
                    }
                }
            }
        }
    }
    save_registry(app, &accounts)?;
    if records_changed {
        save_managed_agents(app, &records)?;
    }
    if definitions_changed {
        save_personas(app, &definitions)?;
    }
    if global_changed {
        save_global_agent_config(app, &global)?;
    }
    std::fs::write(&marker, b"v1\n")
        .map_err(|e| format!("failed to write {}: {e}", marker.display()))?;
    Ok(())
}

pub fn migrate_plaintext_claude_tokens_at_startup(app: &AppHandle) {
    if let Err(error) = migrate_plaintext_claude_tokens(app) {
        eprintln!("buzz-desktop: Claude account migration failed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{claude_token_fingerprint, normalize_claude_token, take_legacy_tokens};

    #[test]
    fn token_normalization_accepts_wrapped_paste() {
        let raw = format!("{}\n  {}", "a".repeat(79), "b".repeat(29));
        let normalized = normalize_claude_token(&raw).expect("wrapped token should normalize");
        assert_eq!(normalized.len(), 108);
        assert!(!normalized.contains(char::is_whitespace));
    }

    #[test]
    fn token_normalization_rejects_wrong_length_and_charset() {
        assert!(normalize_claude_token("short").is_err());
        assert!(normalize_claude_token(&format!("{}!", "a".repeat(107))).is_err());
    }

    #[test]
    fn migration_extracts_canonical_and_misnamed_tokens_without_leaving_plaintext() {
        let canonical = "a".repeat(108);
        let misnamed = "b".repeat(108);
        let mut env = BTreeMap::from([
            ("SECOND_CLAUDE_CODE_OAUTH_TOKEN".into(), misnamed.clone()),
            ("CLAUDE_CODE_OAUTH_TOKEN".into(), canonical.clone()),
            ("SAFE_VALUE".into(), "kept".into()),
        ]);

        let extracted = take_legacy_tokens(&mut env);

        assert_eq!(extracted, vec![canonical, misnamed]);
        assert_eq!(env, BTreeMap::from([("SAFE_VALUE".into(), "kept".into())]));
    }

    #[test]
    fn fingerprint_is_the_sha256_16_contract() {
        assert_eq!(
            claude_token_fingerprint(&"a".repeat(108)),
            "0cef0da81c8b79cf"
        );
    }
}
