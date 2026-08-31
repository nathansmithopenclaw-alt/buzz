use std::path::Path;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use super::{
    config_path, host_id_to_retract, retain_host_retractions, save_config, DeviceRole,
    FleetVisibilityConfig,
};
use crate::app_state::AppState;

const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Transition {
    version: u32,
    target_role: DeviceRole,
}

fn path<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<std::path::PathBuf, String> {
    Ok(config_path(app)?.with_file_name("fleet-visibility-transition.json"))
}

fn load_path(path: &Path) -> Result<Option<Transition>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read Fleet Visibility transition: {error}"))?;
    let transition: Transition = serde_json::from_str(&raw)
        .map_err(|error| format!("failed to parse Fleet Visibility transition: {error}"))?;
    if transition.version != VERSION {
        return Err(format!(
            "unsupported Fleet Visibility transition version {}",
            transition.version
        ));
    }
    if transition.target_role != DeviceRole::Observer {
        return Err("unsupported Fleet Visibility transition target".to_string());
    }
    Ok(Some(transition))
}

fn save_path(path: &Path, transition: Transition) -> Result<(), String> {
    let payload = serde_json::to_vec_pretty(&transition)
        .map_err(|error| format!("failed to serialize Fleet Visibility transition: {error}"))?;
    super::super::storage::atomic_write_json_restricted(path, &payload)
}

fn clear_path(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to clear Fleet Visibility transition: {error}"
        )),
    }
}

pub(super) fn clear<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    clear_path(&path(app)?)
}

fn apply(
    config: &mut FleetVisibilityConfig,
    transition: Transition,
    retract: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    if transition.version != VERSION || transition.target_role != DeviceRole::Observer {
        return Err("unsupported Fleet Visibility transition".to_string());
    }
    if let Some(host_id) = host_id_to_retract(config, DeviceRole::Observer) {
        retract(host_id)?;
    }
    config.role = DeviceRole::Observer;
    config.role_selected = true;
    Ok(())
}

pub(super) fn recover<R: tauri::Runtime>(
    app: &AppHandle<R>,
    config: &mut FleetVisibilityConfig,
) -> Result<(), String> {
    let Some(transition) = load_path(&path(app)?)? else {
        return Ok(());
    };
    let state = app.state::<AppState>();
    apply(config, transition, |host_id| {
        retain_host_retractions(app, &state, host_id)
    })?;
    save_config(app, config)?;

    // A process kill can leave children from the dead Desktop instance alive.
    // Observer recovery must finish the operational half of the transition,
    // not merely prevent a relaunch. The instance marker makes this sweep
    // ownership-safe: live foreign Desktop instances are left untouched.
    let reports =
        super::super::reap_dead_instance_agents(&super::super::current_instance_id(app), &[]);
    if !reports.is_empty() {
        super::super::emit_dead_instance_reap_reports(app, reports);
    }
    clear(app)?;
    eprintln!("buzz-desktop: completed interrupted Fleet Visibility Observer transition");
    Ok(())
}

/// Durably record Observer intent before stopping any runtime. The role mutex
/// serializes this journal write with heartbeat retention. During the shared
/// process grace window, the command holds the runtime-map mutex; start/stop,
/// restore, reconcile, and heartbeat snapshots therefore cannot mutate or
/// observe a partial runtime map. A heartbeat that began before this marker may
/// still finish as Host, but final role commit (or boot recovery) takes the same
/// role mutex and retracts the latest Host heads afterward.
pub(crate) fn begin<R: tauri::Runtime>(app: &AppHandle<R>, state: &AppState) -> Result<(), String> {
    let guard = state
        .fleet_visibility
        .lock()
        .map_err(|error| error.to_string())?;
    if !guard.enabled || !guard.config.role_selected || guard.config.role != DeviceRole::Host {
        return Ok(());
    }
    save_path(
        &path(app)?,
        Transition {
            version: VERSION,
            target_role: DeviceRole::Observer,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn killed_mid_transition_rolls_forward_to_observer_on_recovery() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("fleet-visibility-transition.json");
        let mut config = FleetVisibilityConfig {
            role: DeviceRole::Host,
            role_selected: true,
            host_id: Some("host-a".into()),
            ..FleetVisibilityConfig::default()
        };
        save_path(
            &path,
            Transition {
                version: VERSION,
                target_role: DeviceRole::Observer,
            },
        )
        .unwrap();

        // Simulate the Desktop process being killed here: the next process
        // starts with Host still committed and only the durable intent file.
        let transition = load_path(&path).unwrap().unwrap();
        let mut retracted = None;
        apply(&mut config, transition, |host_id| {
            retracted = Some(host_id.to_string());
            Ok(())
        })
        .unwrap();

        assert_eq!(retracted.as_deref(), Some("host-a"));
        assert_eq!(config.role, DeviceRole::Observer);
        assert!(config.role_selected);
        clear_path(&path).unwrap();
        assert_eq!(load_path(&path).unwrap(), None);
    }
}
