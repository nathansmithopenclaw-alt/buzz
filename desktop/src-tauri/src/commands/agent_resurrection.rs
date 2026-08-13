//! Tauri commands that resurrect a stopped managed-agent harness.
//!
//! Decision logic lives in `managed_agents::resurrection`. These commands
//! load live process state and call `start_managed_agent_inner` (no
//! crash-loop clear — that is owner-start only).

use std::collections::HashSet;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::app_state::AppState;
use crate::managed_agents::{
    current_instance_id, decide_apply_restart, decide_inbound_respawn, load_managed_agents,
    note_automatic_spawn, save_managed_agents, sync_managed_agent_processes, ApplyRestartDecision,
    BackendKind, InboundRespawnDecision,
};

use super::agents::{start_managed_agent_inner, workspace_owner_hex};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboundRespawnResult {
    pub pubkey: String,
    pub spawned: bool,
    pub reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyRestartResult {
    pub started: Vec<String>,
    pub failed: Vec<String>,
}

fn local_sibling_pubkeys(records: &[crate::managed_agents::ManagedAgentRecord]) -> HashSet<String> {
    records
        .iter()
        .filter(|record| record.is_active && record.backend == BackendKind::Local)
        .map(|record| record.pubkey.clone())
        .collect()
}

fn agent_is_running(
    runtimes: &mut std::collections::HashMap<
        crate::managed_agents::ManagedAgentRuntimeKey,
        crate::managed_agents::ManagedAgentPairRuntime,
    >,
    pubkey: &str,
) -> bool {
    runtimes.iter_mut().any(|(key, runtime)| {
        key.pubkey.eq_ignore_ascii_case(pubkey) && runtime.child.try_wait().ok().flatten().is_none()
    })
}

/// Spawn a stopped local active agent if this inbound event is one it would answer.
#[tauri::command]
pub async fn ensure_managed_agent_for_inbound(
    pubkey: String,
    author_pubkey: String,
    is_dm: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<InboundRespawnResult, String> {
    let owner_hex = workspace_owner_hex(&state).ok();
    let (decision, spawn_pubkey) = {
        let _store = state
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
        for exited in &exited_pubkeys {
            state.clear_agent_session_caches(exited);
        }
        let Some(record) = records
            .iter()
            .find(|record| record.pubkey.eq_ignore_ascii_case(&pubkey))
        else {
            return Ok(InboundRespawnResult {
                pubkey,
                spawned: false,
                reason: "unknown_agent".into(),
            });
        };
        let running = agent_is_running(&mut runtimes, &pubkey);
        let siblings = local_sibling_pubkeys(&records);
        let decision = decide_inbound_respawn(
            record,
            &author_pubkey,
            owner_hex.as_deref(),
            &siblings,
            running,
            is_dm,
        );
        (decision, record.pubkey.clone())
    };

    match decision {
        InboundRespawnDecision::Spawn => {
            match start_managed_agent_inner(spawn_pubkey.clone(), app, state).await {
                Ok(_) => {
                    note_automatic_spawn(&spawn_pubkey);
                    Ok(InboundRespawnResult {
                        pubkey: spawn_pubkey,
                        spawned: true,
                        reason: "spawned".into(),
                    })
                }
                Err(error) => Ok(InboundRespawnResult {
                    pubkey: spawn_pubkey,
                    spawned: false,
                    reason: error,
                }),
            }
        }
        other => Ok(InboundRespawnResult {
            pubkey: spawn_pubkey,
            spawned: false,
            reason: format!("{other:?}"),
        }),
    }
}

/// After apply-update writes config, start every stopped active local instance
/// linked to `persona_id` (or the single `pubkey` when given).
#[tauri::command]
pub async fn resurrect_stopped_active_instances(
    persona_id: Option<String>,
    pubkey: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ApplyRestartResult, String> {
    let candidates = {
        let _store = state
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
        for exited in &exited_pubkeys {
            state.clear_agent_session_caches(exited);
        }
        records
            .iter()
            .filter(|record| {
                if let Some(pubkey) = pubkey.as_deref() {
                    record.pubkey.eq_ignore_ascii_case(pubkey)
                } else if let Some(persona_id) = persona_id.as_deref() {
                    record.persona_id.as_deref() == Some(persona_id)
                } else {
                    false
                }
            })
            .filter_map(|record| {
                let running = agent_is_running(&mut runtimes, &record.pubkey);
                match decide_apply_restart(record, running) {
                    ApplyRestartDecision::Start => Some(record.pubkey.clone()),
                    _ => None,
                }
            })
            .collect::<Vec<_>>()
    };

    let mut started = Vec::new();
    let mut failed = Vec::new();
    for candidate in candidates {
        let spawn_state = app.state::<AppState>();
        match start_managed_agent_inner(candidate.clone(), app.clone(), spawn_state).await {
            Ok(_) => {
                note_automatic_spawn(&candidate);
                started.push(candidate);
            }
            Err(error) => failed.push(format!("{candidate}: {error}")),
        }
    }
    Ok(ApplyRestartResult { started, failed })
}
