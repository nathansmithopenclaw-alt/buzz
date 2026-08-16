use tauri::{AppHandle, Manager, State};

use crate::{
    app_state::AppState,
    managed_agents::{
        fleet_visibility::{self, FleetVisibilitySettings},
        load_managed_agents, restore_managed_agents_on_launch, save_managed_agents,
        stop_managed_agent_process, BackendKind, DeviceRole,
    },
};

#[tauri::command]
pub fn get_fleet_visibility_settings(
    state: State<'_, AppState>,
) -> Result<FleetVisibilitySettings, String> {
    fleet_visibility::settings(&state.fleet_visibility)
}

/// Pubkeys for local instance records whose agent secret is available now.
/// The first-run chooser must not mistake a stale key-less record for a Host
/// capable of running the matching relay projection.
#[tauri::command]
pub async fn list_keyed_managed_agent_pubkeys(app: AppHandle) -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        Ok(load_managed_agents(&app)?
            .into_iter()
            .filter(fleet_visibility::record_has_matching_key)
            .map(|record| record.pubkey)
            .collect())
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))?
}

fn resume_deferred_restore_if_allowed(app: &AppHandle, state: &AppState) {
    if fleet_visibility::require_host(state).is_err() {
        return;
    }
    if !state
        .managed_agent_restore_pending
        .swap(false, std::sync::atomic::Ordering::AcqRel)
    {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        if let Err(error) = restore_managed_agents_on_launch(&app, &state.shutdown_started).await {
            eprintln!("buzz-desktop: deferred fleet-role restore failed: {error}");
        }
    });
}

#[tauri::command]
pub fn set_fleet_visibility_enabled(
    enabled: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<FleetVisibilitySettings, String> {
    fleet_visibility::set_enabled(&app, &state, enabled)?;
    if !enabled {
        resume_deferred_restore_if_allowed(&app, &state);
    }
    fleet_visibility::settings(&state.fleet_visibility)
}

fn stop_local_agents_for_observer(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = load_managed_agents(app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;
    let mut changed = false;
    for record in records
        .iter_mut()
        .filter(|record| record.backend == BackendKind::Local)
    {
        // Drain every local record, not just the in-memory runtime map. The
        // stop helper also clears legacy pid files and restored receipts, so
        // Observer selection cannot leave an untracked local child behind.
        stop_managed_agent_process(app, record, &mut runtimes)?;
        changed = true;
    }
    if changed {
        save_managed_agents(app, &records)?;
    }
    Ok(())
}

#[tauri::command]
pub fn set_fleet_visibility_role(
    role: DeviceRole,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<FleetVisibilitySettings, String> {
    match role {
        DeviceRole::Observer => {
            stop_local_agents_for_observer(&app, &state)?;
            let settings = fleet_visibility::set_role(&app, &state, role)?;
            state
                .managed_agent_restore_pending
                .store(true, std::sync::atomic::Ordering::Release);
            eprintln!(
                "buzz-desktop: Fleet Visibility Observer role selected; local restore skipped"
            );
            Ok(settings)
        }
        DeviceRole::Host => {
            let settings = fleet_visibility::set_role(&app, &state, role)?;
            fleet_visibility::retain_host_heartbeat_tick(&app, &state);
            resume_deferred_restore_if_allowed(&app, &state);
            let flush_app = app.clone();
            tauri::async_runtime::spawn(async move {
                let state = flush_app.state::<AppState>();
                if let Err(error) =
                    crate::managed_agents::persona_events::flush_active_pending_events(
                        &flush_app, &state,
                    )
                    .await
                {
                    eprintln!("buzz-desktop: Host role heartbeat flush failed: {error}");
                }
            });
            Ok(settings)
        }
    }
}
