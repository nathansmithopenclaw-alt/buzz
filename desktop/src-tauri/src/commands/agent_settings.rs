use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager, State};

use crate::{
    app_state::AppState,
    managed_agents::{
        current_instance_id, find_managed_agent_mut, load_managed_agents, save_managed_agents,
        sync_managed_agent_processes, ManagedAgentSummary,
    },
    util::now_iso,
};

#[tauri::command]
pub fn set_agent_managed_profiles(enabled: bool, state: State<'_, AppState>) {
    state
        .managed_agent_profile_reconcile_enabled()
        .store(!enabled, Ordering::Release);
}

#[tauri::command]
pub fn set_thread_scoped_acp_sessions(enabled: bool, state: State<'_, AppState>) {
    state
        .thread_scoped_acp_sessions_enabled()
        .store(enabled, Ordering::Release);
}

#[tauri::command]
pub fn set_agent_session_residency(enabled: bool, state: State<'_, AppState>) {
    state
        .session_residency_enabled()
        .store(enabled, Ordering::Release);
}

#[tauri::command]
pub fn set_answered_event_watermark(enabled: bool, state: State<'_, AppState>) {
    state
        .answered_event_watermark_enabled()
        .store(enabled, Ordering::Release);
}

#[tauri::command]
pub async fn set_managed_agent_start_on_app_launch(
    pubkey: String,
    start_on_app_launch: bool,
    app: AppHandle,
) -> Result<ManagedAgentSummary, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        crate::managed_agents::require_fleet_host(&state)?;
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut records = load_managed_agents(&app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|error| error.to_string())?;

        let (sync_changed, exited_pubkeys) =
            sync_managed_agent_processes(&mut records, &mut runtimes, &current_instance_id(&app));
        if sync_changed {
            save_managed_agents(&app, &records)?;
        }
        for pubkey in &exited_pubkeys {
            state.clear_agent_session_caches(pubkey);
        }

        {
            let record = find_managed_agent_mut(&mut records, &pubkey)?;
            record.start_on_app_launch = start_on_app_launch;
            record.updated_at = now_iso();
        }

        save_managed_agents(&app, &records)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        super::agents::summarize_from_disk(&app, record, &runtimes)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn set_managed_agent_auto_restart(
    pubkey: String,
    auto_restart_on_config_change: bool,
    app: AppHandle,
) -> Result<ManagedAgentSummary, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        crate::managed_agents::require_fleet_host(&state)?;
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut records = load_managed_agents(&app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|error| error.to_string())?;

        let (sync_changed, exited_pubkeys) =
            sync_managed_agent_processes(&mut records, &mut runtimes, &current_instance_id(&app));
        if sync_changed {
            save_managed_agents(&app, &records)?;
        }
        for pubkey in &exited_pubkeys {
            state.clear_agent_session_caches(pubkey);
        }

        {
            let record = find_managed_agent_mut(&mut records, &pubkey)?;
            record.auto_restart_on_config_change = auto_restart_on_config_change;
            record.updated_at = now_iso();
        }

        save_managed_agents(&app, &records)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        super::agents::summarize_from_disk(&app, record, &runtimes)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn set_managed_agent_session_residency_policy(
    pubkey: String,
    idle_pool_sleep_seconds: Option<u64>,
    max_turns_per_session: Option<u32>,
    app: AppHandle,
) -> Result<ManagedAgentSummary, String> {
    const MAX_IDLE_POOL_SLEEP_SECONDS: u64 = 30 * 24 * 60 * 60;
    const MAX_TURNS_PER_SESSION: u32 = 10_000;

    if idle_pool_sleep_seconds.is_some_and(|seconds| seconds > MAX_IDLE_POOL_SLEEP_SECONDS) {
        return Err(format!(
            "idle pool sleep must be at most {MAX_IDLE_POOL_SLEEP_SECONDS} seconds"
        ));
    }
    if max_turns_per_session.is_some_and(|turns| turns > MAX_TURNS_PER_SESSION) {
        return Err(format!(
            "max turns per session must be at most {MAX_TURNS_PER_SESSION}"
        ));
    }

    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        crate::managed_agents::require_fleet_host(&state)?;
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut records = load_managed_agents(&app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|error| error.to_string())?;

        let (sync_changed, exited_pubkeys) =
            sync_managed_agent_processes(&mut records, &mut runtimes, &current_instance_id(&app));
        if sync_changed {
            save_managed_agents(&app, &records)?;
        }
        for pubkey in &exited_pubkeys {
            state.clear_agent_session_caches(pubkey);
        }

        {
            let record = find_managed_agent_mut(&mut records, &pubkey)?;
            record.idle_pool_sleep_seconds = idle_pool_sleep_seconds;
            record.max_turns_per_session = max_turns_per_session;
            record.updated_at = now_iso();
        }

        save_managed_agents(&app, &records)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        super::agents::summarize_from_disk(&app, record, &runtimes)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}
