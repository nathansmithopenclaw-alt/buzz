//! Device-local Fleet Visibility role and Host liveness publication.
//!
//! The role file lives in the app data root, never in `managed-agents.json`,
//! and is never synchronized. Rust is the authority for every lifecycle and
//! creation boundary; frontend hiding is only defense in depth.

use std::{collections::HashSet, sync::Mutex};

use nostr::{EventBuilder, JsonUtil, Kind, Tag};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use super::{
    persona_events::monotonic_created_at,
    retention::{
        delete_retained_event, get_retained_event, open_retention_db, retain_event,
        tombstone_retention_d_tag, RetainedEvent,
    },
    BackendKind, ManagedAgentRecord,
};
use crate::app_state::AppState;

pub const KIND_HOST_DECLARATION: u32 = 30_180;
pub const KIND_HOST_STATUS: u32 = 30_181;
const CONFIG_VERSION: u32 = 1;
const MAX_STATUS_AGENTS: usize = 256;

pub const OBSERVER_REFUSAL: &str =
    "Observer device cannot control agents; switch this device to Host in Settings";
pub const ROLE_CHOICE_REFUSAL: &str =
    "Choose whether this device is a Host or Observer before controlling agents";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceRole {
    #[default]
    Host,
    Observer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FleetVisibilityConfig {
    #[serde(default = "config_version")]
    version: u32,
    #[serde(default)]
    role: DeviceRole,
    #[serde(default)]
    role_selected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host_created_at: Option<u64>,
    #[serde(default)]
    last_seq: u64,
}

impl Default for FleetVisibilityConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            role: DeviceRole::Host,
            role_selected: false,
            host_id: None,
            host_name: None,
            host_created_at: None,
            last_seq: 0,
        }
    }
}

const fn config_version() -> u32 {
    CONFIG_VERSION
}

#[derive(Debug, Clone, Default)]
pub struct FleetVisibilityRuntime {
    enabled: bool,
    config: FleetVisibilityConfig,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetVisibilitySettings {
    pub enabled: bool,
    pub role: DeviceRole,
    pub needs_choice: bool,
    pub host_id: Option<String>,
    pub host_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HostAgentState {
    Running,
    Stopped,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostAgentStatus {
    pub pubkey: String,
    pub state: HostAgentState,
    pub since: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostDeclarationContent {
    pub v: u8,
    pub name: String,
    pub platform: &'static str,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostStatusContent {
    pub v: u8,
    pub seq: u64,
    pub ts: u64,
    pub agents: Vec<HostAgentStatus>,
}

fn config_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data dir: {error}"))?;
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create app data dir: {error}"))?;
    Ok(dir.join("fleet-visibility.json"))
}

fn load_config(app: &AppHandle) -> Result<FleetVisibilityConfig, String> {
    let path = config_path(app)?;
    if !path.exists() {
        return Ok(FleetVisibilityConfig::default());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| format!("failed to read Fleet Visibility config: {error}"))?;
    let config: FleetVisibilityConfig = serde_json::from_str(&raw)
        .map_err(|error| format!("failed to parse Fleet Visibility config: {error}"))?;
    if config.version != CONFIG_VERSION {
        return Err(format!(
            "unsupported Fleet Visibility config version {}",
            config.version
        ));
    }
    Ok(config)
}

fn save_config(app: &AppHandle, config: &FleetVisibilityConfig) -> Result<(), String> {
    let payload = serde_json::to_vec_pretty(config)
        .map_err(|error| format!("failed to serialize Fleet Visibility config: {error}"))?;
    super::storage::atomic_write_json_restricted(&config_path(app)?, &payload)
}

pub fn configure_runtime(
    app: &AppHandle,
    runtime: &Mutex<FleetVisibilityRuntime>,
    enabled: bool,
) -> Result<(), String> {
    if !enabled {
        let mut guard = runtime.lock().map_err(|error| error.to_string())?;
        guard.enabled = false;
        return Ok(());
    }
    let config = load_config(app)?;
    let mut guard = runtime.lock().map_err(|error| error.to_string())?;
    guard.enabled = enabled;
    guard.config = config;
    Ok(())
}

pub fn settings(
    runtime: &Mutex<FleetVisibilityRuntime>,
) -> Result<FleetVisibilitySettings, String> {
    let guard = runtime.lock().map_err(|error| error.to_string())?;
    Ok(FleetVisibilitySettings {
        enabled: guard.enabled,
        role: guard.config.role,
        needs_choice: guard.enabled && !guard.config.role_selected,
        host_id: guard.config.host_id.clone(),
        host_name: guard.config.host_name.clone(),
    })
}

/// Rust-side authority check. Experiment-off is exactly today's behavior.
pub fn require_host(state: &AppState) -> Result<(), String> {
    let guard = state
        .fleet_visibility
        .lock()
        .map_err(|error| error.to_string())?;
    host_authority(&guard)
}

fn host_authority(runtime: &FleetVisibilityRuntime) -> Result<(), String> {
    if !runtime.enabled {
        return Ok(());
    }
    if !runtime.config.role_selected {
        return Err(ROLE_CHOICE_REFUSAL.to_string());
    }
    match runtime.config.role {
        DeviceRole::Host => Ok(()),
        DeviceRole::Observer => Err(OBSERVER_REFUSAL.to_string()),
    }
}

pub fn is_observer(state: &AppState) -> bool {
    state
        .fleet_visibility
        .lock()
        .map(|guard| {
            guard.enabled && guard.config.role_selected && guard.config.role == DeviceRole::Observer
        })
        .unwrap_or(true)
}

pub fn is_pending_choice(state: &AppState) -> bool {
    state
        .fleet_visibility
        .lock()
        .map(|guard| guard.enabled && !guard.config.role_selected)
        .unwrap_or(true)
}

pub fn is_host(state: &AppState) -> bool {
    state
        .fleet_visibility
        .lock()
        .map(|guard| {
            guard.enabled && guard.config.role_selected && guard.config.role == DeviceRole::Host
        })
        .unwrap_or(false)
}

pub fn record_has_matching_key(record: &ManagedAgentRecord) -> bool {
    nostr::Keys::parse(record.private_key_nsec.trim()).is_ok_and(|keys| {
        keys.public_key()
            .to_hex()
            .eq_ignore_ascii_case(record.pubkey.trim())
    })
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(1)
        .max(1)
}

fn default_host_name() -> String {
    for key in ["COMPUTERNAME", "HOSTNAME"] {
        if let Some(value) = std::env::var_os(key) {
            let value = sanitize_host_name(&value.to_string_lossy());
            if !value.is_empty() {
                return value;
            }
        }
    }
    std::process::Command::new("hostname")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| sanitize_host_name(&value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "This device".to_string())
}

fn sanitize_host_name(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(128)
        .collect()
}

fn platform_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

fn parse_record_timestamp(value: Option<&str>, fallback: u64) -> u64 {
    value
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp().max(1) as u64)
        .unwrap_or(fallback)
}

fn record_timestamp(value: Option<&str>) -> Option<i64> {
    value
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp())
}

fn canonical_curve_pubkey(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() != 64
        || !value
            .chars()
            .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
    {
        return None;
    }
    let pubkey = nostr::PublicKey::from_hex(value).ok()?;
    pubkey.xonly().ok()?;
    Some(value.to_string())
}

pub fn project_agent_statuses(
    records: &[ManagedAgentRecord],
    local_running_pubkeys: Option<&HashSet<String>>,
    now: u64,
) -> Vec<HostAgentStatus> {
    let mut agents = records
        .iter()
        .filter_map(|record| {
            let canonical_pubkey = canonical_curve_pubkey(&record.pubkey)?;
            let running = match &record.backend {
                BackendKind::Local => local_running_pubkeys.map_or_else(
                    || {
                        record.runtime_pid.is_some()
                            || (record.last_error.is_none()
                                && record_timestamp(record.last_started_at.as_deref())
                                    > record_timestamp(record.last_stopped_at.as_deref()))
                    },
                    |running| running.contains(&record.pubkey),
                ),
                BackendKind::Provider { .. } => record.backend_agent_id.is_some(),
            };
            let state = if record.last_error.is_some() {
                HostAgentState::Error
            } else if running {
                HostAgentState::Running
            } else {
                HostAgentState::Stopped
            };
            let created = parse_record_timestamp(Some(&record.created_at), now)
                .min(now)
                .max(1);
            let since = match state {
                HostAgentState::Running => {
                    parse_record_timestamp(record.last_started_at.as_deref(), created)
                }
                HostAgentState::Stopped => {
                    parse_record_timestamp(record.last_stopped_at.as_deref(), created)
                }
                HostAgentState::Error => parse_record_timestamp(Some(&record.updated_at), created),
            }
            .min(now)
            .max(1);
            Some(HostAgentStatus {
                pubkey: canonical_pubkey,
                state,
                since,
            })
        })
        .collect::<Vec<_>>();
    agents.sort_by(|left, right| left.pubkey.cmp(&right.pubkey));
    agents.dedup_by(|left, right| left.pubkey == right.pubkey);
    agents.truncate(MAX_STATUS_AGENTS);
    agents
}

fn retain_host_event(
    app: &AppHandle,
    state: &AppState,
    kind: u32,
    host_id: &str,
    content: String,
) -> Result<(), String> {
    let scope = super::retention::active_retention_scope(app, state)?;
    let owner = scope.owner_keys.public_key().to_hex();
    let conn = open_retention_db(&scope.db_path)?;
    let existing = get_retained_event(&conn, kind, &owner, host_id)?;
    if kind == KIND_HOST_DECLARATION
        && existing
            .as_ref()
            .is_some_and(|retained| retained.content == content)
    {
        return Ok(());
    }
    let d_tag =
        Tag::parse(["d", host_id]).map_err(|error| format!("invalid host d-tag: {error}"))?;
    let event = EventBuilder::new(Kind::Custom(kind as u16), content)
        .tags([d_tag])
        .custom_created_at(monotonic_created_at(
            existing.as_ref().map(|row| row.created_at),
        ))
        .sign_with_keys(&scope.owner_keys)
        .map_err(|error| format!("failed to sign host event: {error}"))?;
    retain_event(
        &conn,
        &RetainedEvent {
            kind,
            pubkey: owner,
            d_tag: host_id.to_string(),
            content: event.content.clone(),
            created_at: event.created_at.as_secs() as i64,
            raw_event: event.as_json(),
            pending_sync: true,
        },
    )
}

/// Build one coordinate deletion for one Host projection kind.
///
/// The relay's NIP-09 path requires exactly one `a` tag per event, so 30180
/// and 30181 deliberately use separate signed events.
fn build_host_delete(
    target_kind: u32,
    host_id: &str,
    owner_pubkey_hex: &str,
) -> Result<EventBuilder, String> {
    let coord = format!("{target_kind}:{owner_pubkey_hex}:{host_id}");
    let tag = Tag::parse(["a", coord.as_str()])
        .map_err(|error| format!("invalid host deletion a-tag: {error}"))?;
    Ok(EventBuilder::new(Kind::Custom(5), "").tags([tag]))
}

/// Replace both retained Host projections with durable pending tombstones.
/// Offline publication is safe: the normal pending-event loop retries these
/// rows after reconnect and always orders kind:5 before replacement events.
fn retain_host_retractions(app: &AppHandle, state: &AppState, host_id: &str) -> Result<(), String> {
    let scope = super::retention::active_retention_scope(app, state)?;
    let mut conn = open_retention_db(&scope.db_path)?;
    retain_host_retractions_in_db(&mut conn, &scope.owner_keys, host_id)
}

fn retain_host_retractions_in_db(
    conn: &mut rusqlite::Connection,
    owner_keys: &nostr::Keys,
    host_id: &str,
) -> Result<(), String> {
    let owner = owner_keys.public_key().to_hex();
    let mut tombstones = Vec::with_capacity(2);

    for target_kind in [KIND_HOST_DECLARATION, KIND_HOST_STATUS] {
        let target = get_retained_event(&conn, target_kind, &owner, host_id)?;
        let tombstone_d_tag = tombstone_retention_d_tag(target_kind, host_id);
        let previous_tombstone = get_retained_event(&conn, 5, &owner, &tombstone_d_tag)?;
        let previous_created_at = target
            .as_ref()
            .map(|row| row.created_at)
            .into_iter()
            .chain(previous_tombstone.as_ref().map(|row| row.created_at))
            .max();
        let event = build_host_delete(target_kind, host_id, &owner)?
            .custom_created_at(monotonic_created_at(previous_created_at))
            .sign_with_keys(owner_keys)
            .map_err(|error| format!("failed to sign host deletion: {error}"))?;
        tombstones.push((target_kind, tombstone_d_tag, event));
    }

    let transaction = conn
        .transaction()
        .map_err(|error| format!("failed to begin host retraction transaction: {error}"))?;
    for (target_kind, tombstone_d_tag, event) in tombstones {
        delete_retained_event(&transaction, target_kind, &owner, host_id)?;
        retain_event(
            &transaction,
            &RetainedEvent {
                kind: 5,
                pubkey: owner.clone(),
                d_tag: tombstone_d_tag,
                content: event.content.clone(),
                created_at: event.created_at.as_secs() as i64,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )?;
    }
    transaction
        .commit()
        .map_err(|error| format!("failed to commit host retractions: {error}"))
}

fn host_id_to_retract(config: &FleetVisibilityConfig, role: DeviceRole) -> Option<&str> {
    (config.role_selected && config.role == DeviceRole::Host && role == DeviceRole::Observer)
        .then(|| config.host_id.as_deref())
        .flatten()
}

/// Queue a Host declaration and a fresh status heartbeat in the existing
/// pending-sync store. Call before each 30-second flush and after lifecycle
/// writes; Observer/disabled devices are true no-ops.
pub fn retain_host_heartbeat(
    app: &AppHandle,
    state: &AppState,
    records: &[ManagedAgentRecord],
) -> Result<bool, String> {
    retain_host_heartbeat_with_running(app, state, records, None)
}

/// Publish using the in-memory runtime map as the authoritative local running
/// set. Callers use this at boot and on keepalive; store-write hooks use the
/// record timestamps because they often already hold the runtime mutex.
pub fn retain_host_heartbeat_with_runtime_snapshot(
    app: &AppHandle,
    state: &AppState,
    records: &[ManagedAgentRecord],
) -> Result<bool, String> {
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;
    let running = runtimes
        .iter_mut()
        .filter_map(|(key, runtime)| match runtime.child.try_wait() {
            Ok(None) => Some(key.pubkey.clone()),
            Ok(Some(_)) | Err(_) => None,
        })
        .collect::<HashSet<_>>();
    drop(runtimes);
    retain_host_heartbeat_with_running(app, state, records, Some(&running))
}

pub fn retain_host_heartbeat_tick(app: &AppHandle, state: &AppState) {
    if !is_host(state) {
        return;
    }
    match super::load_managed_agents(app) {
        Ok(records) => {
            if let Err(error) = retain_host_heartbeat_with_runtime_snapshot(app, state, &records) {
                eprintln!("buzz-desktop: host-heartbeat-retain: {error}");
            }
        }
        Err(error) => eprintln!("buzz-desktop: host-heartbeat-load: {error}"),
    }
}

pub fn retain_after_agent_store_write(app: &AppHandle, records: &[ManagedAgentRecord]) {
    if let Some(state) = app.try_state::<AppState>() {
        if let Err(error) = retain_host_heartbeat(app, &state, records) {
            eprintln!("buzz-desktop: host-heartbeat-after-save: {error}");
        }
    }
}

fn retain_host_heartbeat_with_running(
    app: &AppHandle,
    state: &AppState,
    records: &[ManagedAgentRecord],
    local_running_pubkeys: Option<&HashSet<String>>,
) -> Result<bool, String> {
    let now = unix_now();
    // Keep the role mutex through both retained writes. A Host->Observer
    // transition waits here, then atomically replaces these rows with
    // tombstones; an in-flight heartbeat can never land after retraction.
    let mut guard = state
        .fleet_visibility
        .lock()
        .map_err(|error| error.to_string())?;
    if !guard.enabled || !guard.config.role_selected || guard.config.role != DeviceRole::Host {
        return Ok(false);
    }
    if guard.config.host_id.is_none() {
        guard.config.host_id = Some(uuid::Uuid::new_v4().to_string());
    }
    if guard.config.host_name.is_none() {
        guard.config.host_name = Some(default_host_name());
    }
    if guard.config.host_created_at.is_none() {
        guard.config.host_created_at = Some(now);
    }
    guard.config.last_seq = guard.config.last_seq.saturating_add(1).max(1);
    save_config(app, &guard.config)?;
    let host_id = guard.config.host_id.clone().unwrap_or_default();
    let declaration = HostDeclarationContent {
        v: 1,
        name: guard.config.host_name.clone().unwrap_or_default(),
        platform: platform_name(),
        created_at: guard.config.host_created_at.unwrap_or(now),
    };
    let status = HostStatusContent {
        v: 1,
        seq: guard.config.last_seq,
        ts: now,
        agents: project_agent_statuses(records, local_running_pubkeys, now),
    };
    retain_host_event(
        app,
        state,
        KIND_HOST_DECLARATION,
        &host_id,
        serde_json::to_string(&declaration)
            .map_err(|error| format!("failed to serialize host declaration: {error}"))?,
    )?;
    retain_host_event(
        app,
        state,
        KIND_HOST_STATUS,
        &host_id,
        serde_json::to_string(&status)
            .map_err(|error| format!("failed to serialize host status: {error}"))?,
    )?;
    Ok(true)
}

pub fn set_enabled(app: &AppHandle, state: &AppState, enabled: bool) -> Result<(), String> {
    configure_runtime(app, &state.fleet_visibility, enabled)
}

pub fn set_role(
    app: &AppHandle,
    state: &AppState,
    role: DeviceRole,
) -> Result<FleetVisibilitySettings, String> {
    let mut guard = state
        .fleet_visibility
        .lock()
        .map_err(|error| error.to_string())?;
    let mut config = guard.config.clone();
    if let Some(host_id) = host_id_to_retract(&config, role) {
        retain_host_retractions(app, state, host_id)?;
    }
    config.role = role;
    config.role_selected = true;
    if role == DeviceRole::Host {
        config
            .host_id
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string());
        config.host_name.get_or_insert_with(default_host_name);
        config.host_created_at.get_or_insert_with(unix_now);
    }
    save_config(app, &config)?;
    guard.config = config;
    drop(guard);
    settings(&state.fleet_visibility)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(pubkey: &str) -> ManagedAgentRecord {
        serde_json::from_value(serde_json::json!({
            "pubkey": pubkey,
            "name": "Probe",
            "relay_url": "",
            "acp_command": "",
            "agent_command": "",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 0,
            "system_prompt": null,
            "created_at": "2026-08-16T20:00:00Z",
            "updated_at": "2026-08-16T20:00:00Z",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }))
        .unwrap()
    }

    #[test]
    fn default_is_host_but_requires_choice_only_when_enabled() {
        let mut runtime = FleetVisibilityRuntime::default();
        assert_eq!(runtime.config.role, DeviceRole::Host);
        assert!(!runtime.config.role_selected);
        assert!(host_authority(&runtime).is_ok());
        runtime.enabled = true;
        assert_eq!(host_authority(&runtime).unwrap_err(), ROLE_CHOICE_REFUSAL);
        let mutex = Mutex::new(runtime);
        assert!(settings(&mutex).unwrap().needs_choice);
    }

    #[test]
    fn observer_authority_fails_closed_while_experiment_off_is_a_noop() {
        let mut runtime = FleetVisibilityRuntime::default();
        runtime.config.role = DeviceRole::Observer;
        runtime.config.role_selected = true;
        assert!(host_authority(&runtime).is_ok());
        runtime.enabled = true;
        assert_eq!(host_authority(&runtime).unwrap_err(), OBSERVER_REFUSAL);
    }

    #[test]
    fn keyed_record_requires_a_parseable_matching_secret() {
        let keys = nostr::Keys::generate();
        let mut record = record(&keys.public_key().to_hex());
        record.private_key_nsec = keys.secret_key().to_secret_hex();
        assert!(record_has_matching_key(&record));
        record.pubkey = nostr::Keys::generate().public_key().to_hex();
        assert!(!record_has_matching_key(&record));
        record.private_key_nsec.clear();
        assert!(!record_has_matching_key(&record));
    }

    #[test]
    fn host_name_sanitization_enforces_schema_bounds() {
        let raw = format!("  Mini\u{0000}{}  ", "x".repeat(200));
        let sanitized = sanitize_host_name(&raw);
        assert!(!sanitized.chars().any(char::is_control));
        assert_eq!(sanitized.chars().count(), 128);
    }

    #[test]
    fn status_projection_is_lifecycle_only_sorted_and_capped() {
        let now = 1_776_000_000;
        let mut records = (0..MAX_STATUS_AGENTS)
            .map(|_| record(&nostr::Keys::generate().public_key().to_hex()))
            .collect::<Vec<_>>();
        records[0].runtime_pid = Some(7);
        records[1].last_error = Some("boom".into());
        let projected = project_agent_statuses(&records, None, now);
        assert_eq!(projected.len(), MAX_STATUS_AGENTS);
        assert_eq!(
            projected
                .iter()
                .filter(|agent| agent.state == HostAgentState::Running)
                .count(),
            1
        );
        assert_eq!(
            projected
                .iter()
                .filter(|agent| agent.state == HostAgentState::Error)
                .count(),
            1
        );
        assert!(projected
            .windows(2)
            .all(|pair| pair[0].pubkey < pair[1].pubkey));
        let json = serde_json::to_value(HostStatusContent {
            v: 1,
            seq: 1,
            ts: now,
            agents: projected,
        })
        .unwrap();
        assert_eq!(
            json.as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["agents", "seq", "ts", "v"]
        );
        assert_eq!(
            json["agents"][0]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["pubkey", "since", "state"]
        );

        let over_cap = (0..(MAX_STATUS_AGENTS + 20))
            .map(|_| record(&nostr::Keys::generate().public_key().to_hex()))
            .collect::<Vec<_>>();
        assert_eq!(
            project_agent_statuses(&over_cap, None, now).len(),
            MAX_STATUS_AGENTS
        );
    }

    #[test]
    fn declaration_matches_locked_exact_key_schema() {
        let value = serde_json::to_value(HostDeclarationContent {
            v: 1,
            name: "Mini".into(),
            platform: "macos",
            created_at: 1,
        })
        .unwrap();
        assert_eq!(
            value
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["created_at", "name", "platform", "v"]
        );
    }

    #[test]
    fn status_projection_rejects_noncanonical_or_noncurve_pubkeys() {
        let valid = nostr::Keys::generate().public_key().to_hex();
        let records = vec![
            record(&valid),
            record(&valid.to_uppercase()),
            record(&"f".repeat(64)),
        ];
        let projected = project_agent_statuses(&records, None, 1_776_000_000);
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].pubkey, valid);
    }

    #[test]
    fn host_deletes_are_two_single_coordinate_events() {
        const OWNER: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
        let keys = nostr::Keys::generate();
        for target_kind in [KIND_HOST_DECLARATION, KIND_HOST_STATUS] {
            let event = build_host_delete(target_kind, "host-a", OWNER)
                .unwrap()
                .sign_with_keys(&keys)
                .unwrap();
            assert_eq!(event.kind, Kind::Custom(5));
            let a_tags = event
                .tags
                .iter()
                .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("a"))
                .collect::<Vec<_>>();
            assert_eq!(a_tags.len(), 1);
            assert_eq!(
                a_tags[0].as_slice()[1],
                format!("{target_kind}:{OWNER}:host-a")
            );
            assert!(event
                .tags
                .iter()
                .all(|tag| tag.as_slice().first().map(String::as_str) != Some("e")));
        }
    }

    #[test]
    fn only_a_selected_host_to_observer_transition_retracts() {
        let mut config = FleetVisibilityConfig::default();
        config.host_id = Some("host-a".into());
        assert_eq!(host_id_to_retract(&config, DeviceRole::Observer), None);

        config.role_selected = true;
        assert_eq!(
            host_id_to_retract(&config, DeviceRole::Observer),
            Some("host-a")
        );
        assert_eq!(host_id_to_retract(&config, DeviceRole::Host), None);

        config.role = DeviceRole::Observer;
        assert_eq!(host_id_to_retract(&config, DeviceRole::Observer), None);
    }

    #[test]
    fn host_retractions_replace_both_heads_with_pending_tombstones() {
        let tmp = tempfile::tempdir().unwrap();
        let mut conn = open_retention_db(&tmp.path().join("retention.db")).unwrap();
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let host_id = "host-a";

        for kind in [KIND_HOST_DECLARATION, KIND_HOST_STATUS] {
            retain_event(
                &conn,
                &RetainedEvent {
                    kind,
                    pubkey: owner.clone(),
                    d_tag: host_id.into(),
                    content: "{}".into(),
                    created_at: 1,
                    raw_event: "{}".into(),
                    pending_sync: false,
                },
            )
            .unwrap();
        }

        retain_host_retractions_in_db(&mut conn, &keys, host_id).unwrap();

        for kind in [KIND_HOST_DECLARATION, KIND_HOST_STATUS] {
            assert!(get_retained_event(&conn, kind, &owner, host_id)
                .unwrap()
                .is_none());
            let tombstone =
                get_retained_event(&conn, 5, &owner, &tombstone_retention_d_tag(kind, host_id))
                    .unwrap()
                    .unwrap();
            assert!(tombstone.pending_sync);
            let event = nostr::Event::from_json(&tombstone.raw_event).unwrap();
            let a_tags = event
                .tags
                .iter()
                .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("a"))
                .collect::<Vec<_>>();
            assert_eq!(a_tags.len(), 1);
            assert_eq!(a_tags[0].as_slice()[1], format!("{kind}:{owner}:{host_id}"));
        }
    }
}
