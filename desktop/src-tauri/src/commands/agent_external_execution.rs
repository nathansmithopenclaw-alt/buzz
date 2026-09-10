//! Open an already bound external task. This does not start an agent turn.
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

/// Open only the validated target for an existing locally managed identity.
#[tauri::command]
pub fn open_external_agent_task(pubkey: String, app: AppHandle) -> Result<(), String> {
    let state = app.state::<crate::app_state::AppState>();
    crate::managed_agents::require_fleet_host(&state)?;
    let records = crate::managed_agents::load_managed_agents(&app)?;
    let record = records
        .iter()
        .find(|r| r.pubkey.eq_ignore_ascii_case(&pubkey))
        .ok_or("Managed agent not found")?;
    let key = crate::managed_agents::workspace_pair_key(&app, record)
        .ok_or("Agent community is unavailable")?;
    let target = crate::managed_agents::external_execution::load(&app, &key)?
        .ok_or("This agent has no external execution binding in this community")?;
    app.opener()
        .open_url(target.task_url()?, None::<&str>)
        .map_err(|_| "Could not open Codex Desktop. Open the connected task there.".into())
}
