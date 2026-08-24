use tauri::{AppHandle, Manager, State};

use crate::{
    app_state::AppState,
    managed_agents::{
        fleet_visibility::{self, FleetVisibilitySettings},
        load_managed_agents, restore_managed_agents_on_launch, save_managed_agents,
        stop_managed_agent_processes, BackendKind, DeviceRole,
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
    let local_count = records
        .iter()
        .filter(|record| record.backend == BackendKind::Local)
        .count();
    if local_count > 0 {
        // Drain every local record, not just the in-memory runtime map. Signal
        // all runtime pairs before waiting so the fleet shares one bounded
        // graceful-shutdown window instead of paying it once per pair.
        stop_managed_agent_processes(
            app,
            records
                .iter_mut()
                .filter(|record| record.backend == BackendKind::Local),
            &mut runtimes,
        )?;
        changed = true;
    }
    if changed {
        save_managed_agents(app, &records)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn set_fleet_visibility_role(
    role: DeviceRole,
    app: AppHandle,
) -> Result<FleetVisibilitySettings, String> {
    // Stopping a live Host is intentionally blocking process work: every local
    // harness gets a graceful-shutdown window before it is killed. A large
    // fleet can therefore take several seconds to drain. Keep that work off
    // Tauri's command thread so relay events and the rest of the desktop remain
    // responsive throughout the role transition.
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        set_fleet_visibility_role_blocking(role, &app, &state)
    })
    .await
    .map_err(|error| format!("Fleet Visibility role transition task failed: {error}"))?
}

fn set_fleet_visibility_role_blocking(
    role: DeviceRole,
    app: &AppHandle,
    state: &AppState,
) -> Result<FleetVisibilitySettings, String> {
    match role {
        DeviceRole::Observer => {
            fleet_visibility::begin_observer_transition(app, state)?;
            stop_local_agents_for_observer(app, state)?;
            let settings = fleet_visibility::set_role(app, state, role)?;
            state
                .managed_agent_restore_pending
                .store(true, std::sync::atomic::Ordering::Release);
            eprintln!(
                "buzz-desktop: Fleet Visibility Observer role selected; local restore skipped"
            );
            let flush_app = app.clone();
            tauri::async_runtime::spawn(async move {
                let state = flush_app.state::<AppState>();
                if let Err(error) =
                    crate::managed_agents::persona_events::flush_active_pending_events(
                        &flush_app, &state,
                    )
                    .await
                {
                    eprintln!("buzz-desktop: Host retraction flush failed: {error}");
                }
            });
            Ok(settings)
        }
        DeviceRole::Host => {
            let settings = fleet_visibility::set_role(app, state, role)?;
            fleet_visibility::retain_host_heartbeat_tick(app, state);
            resume_deferred_restore_if_allowed(app, state);
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
