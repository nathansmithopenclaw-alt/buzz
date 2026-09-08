//! One durable account operation across this Mac's agents and communities.
mod execute;
mod plan;

use super::account_switch;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
};
use tauri::{AppHandle, Manager};

const MAX_MEMBERS: usize = 512;

fn claude_harness() -> String {
    "claude".into()
}

/// Retry and token renewal preserve enrollment. A different explicit account
/// leaves the default unless the group operation explicitly enrolls it.
pub(crate) fn binding_mode(
    requested: Option<bool>,
    current: bool,
    selected: Option<&str>,
    destination: Option<&str>,
) -> bool {
    requested.unwrap_or(current && selected.is_some() && selected == destination)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ChangeRequest {
    Agent {
        pubkey: String,
        destination: String,
        #[serde(default)]
        inherit_default: bool,
        #[serde(default)]
        all_on_current: bool,
    },
    Default {
        destination: String,
    },
    Remove {
        account_id: String,
        replacement: Option<String>,
    },
}

#[derive(Clone, Serialize, Deserialize)]
struct Member {
    name: String,
    original_account_id: Option<String>,
    original_generation: u64,
    inherit_default: bool,
    mode_only: bool,
    child: account_switch::Operation,
    done: bool,
    error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Operation {
    #[serde(default = "claude_harness")]
    harness: String,
    version: u32,
    id: String,
    request: ChangeRequest,
    source: Option<String>,
    destination: Option<String>,
    destination_minted_at: Option<i64>,
    prior_default: Option<String>,
    change_default: bool,
    remove: bool,
    phase: String,
    error: Option<String>,
    members: Vec<Member>,
}

#[derive(Serialize)]
pub(crate) struct MemberStatus {
    pubkey: String,
    name: String,
    running_count: usize,
    community_count: usize,
    done: bool,
    error: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct Status {
    harness: String,
    id: String,
    phase: String,
    error: Option<String>,
    destination: Option<String>,
    change_default: bool,
    removing: bool,
    members: Vec<MemberStatus>,
}
impl Operation {
    fn status(&self) -> Status {
        Status {
            harness: self.harness.clone(),
            id: self.id.clone(),
            phase: self.phase.clone(),
            error: self.error.clone(),
            destination: self.destination.clone(),
            change_default: self.change_default,
            removing: self.remove,
            members: self
                .members
                .iter()
                .map(|member| MemberStatus {
                    pubkey: member.child.pubkey.clone(),
                    name: member.name.clone(),
                    running_count: if member.mode_only {
                        0
                    } else {
                        member
                            .child
                            .runtimes
                            .iter()
                            .filter(|runtime| runtime.run)
                            .count()
                    },
                    community_count: member.child.runtimes.len(),
                    done: member.done,
                    error: member.error.clone(),
                })
                .collect(),
        }
    }
}

#[derive(Serialize)]
pub(crate) struct Preview {
    fingerprint: String,
    operation: Status,
}

fn worker_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn path(app: &AppHandle) -> Result<PathBuf, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("agents/account-groups");
    super::subscription_binding::private_runtime_dir(&root)?;
    Ok(root.join("active.json"))
}

fn read(app: &AppHandle) -> Result<Option<Operation>, String> {
    let Some(raw) = super::subscription_binding::read_control_file(&path(app)?, 16 * 1024 * 1024)?
    else {
        return Ok(None);
    };
    let operation: Operation =
        serde_json::from_str(&raw).map_err(|_| "Invalid group account journal")?;
    validate(&operation)?;
    for member in &operation.members {
        account_switch::validate_captured(app, &member.child)?;
    }
    Ok(Some(operation))
}

fn validate(operation: &Operation) -> Result<(), String> {
    if !matches!(operation.harness.as_str(), "claude" | "codex")
        || operation.version != if operation.harness == "codex" { 2 } else { 1 }
        || operation.id.len() != 32
        || !operation.id.bytes().all(|c| c.is_ascii_hexdigit())
        || !["applying", "paused", "removing", "complete"].contains(&operation.phase.as_str())
        || operation.members.len() > MAX_MEMBERS
    {
        return Err("Invalid group account journal".into());
    }
    for id in [
        &operation.source,
        &operation.destination,
        &operation.prior_default,
    ]
    .into_iter()
    .flatten()
    {
        if uuid::Uuid::parse_str(id).is_err() {
            return Err("Invalid group account identity".into());
        }
    }
    let mut seen = std::collections::HashSet::new();
    for member in &operation.members {
        if member.child.pubkey.len() != 64
            || !member.child.pubkey.bytes().all(|c| c.is_ascii_hexdigit())
            || !seen.insert(&member.child.pubkey)
            || member.child.runtimes.len() > 64
            || uuid::Uuid::parse_str(&member.child.account_id).is_err()
        {
            return Err("Invalid group account member".into());
        }
    }
    Ok(())
}

fn save(app: &AppHandle, operation: &Operation) -> Result<(), String> {
    validate(operation)?;
    for member in &operation.members {
        account_switch::validate_captured(app, &member.child)?;
    }
    let target = path(app)?;
    let bytes = serde_json::to_vec(operation).map_err(|e| e.to_string())?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("Group account journal exceeds its limit".into());
    }
    super::atomic_write_json_restricted(&target, &bytes)?;
    std::fs::File::open(target.parent().ok_or("Missing group journal directory")?)
        .and_then(|file| file.sync_all())
        .map_err(|e| e.to_string())
}

/// Install each captured child at most once. Explicit Stop can call this before
/// the worker reaches that member; the worker must preserve its updated intent.
pub(crate) fn ensure_child(app: &AppHandle, pubkey: &str) -> Result<(), String> {
    let Some(operation) = read(app)?.filter(|op| op.phase != "complete") else {
        return Ok(());
    };
    let Some(member) = operation
        .members
        .iter()
        .find(|member| member.child.pubkey == pubkey && !member.mode_only)
    else {
        return Ok(());
    };
    if account_switch::read(app, pubkey)?.is_some_and(|child| {
        child.id == member.child.id
            && child.account_id == member.child.account_id
            && child.generation == member.child.generation
    }) {
        return Ok(());
    }
    let records = super::load_managed_agents(app)?;
    let record = records
        .iter()
        .find(|record| record.pubkey == pubkey)
        .ok_or("Group member no longer exists")?;
    if record.subscription_account_generation != member.original_generation
        || record.subscription_account_id != member.original_account_id
    {
        return Err("Group member changed before its account operation was prepared".into());
    }
    account_switch::save(app, &member.child)
}

pub(crate) fn guard_agent_change(
    app: &AppHandle,
    pubkey: &str,
    account_id: Option<&str>,
    group_id: Option<&str>,
) -> Result<(), String> {
    if let Some(op) = read(app)?.filter(|op| op.phase != "complete") {
        if (op
            .members
            .iter()
            .any(|member| member.child.pubkey == pubkey)
            || (account_id.is_some() && op.source.as_deref() == account_id))
            && group_id != Some(op.id.as_str())
        {
            return Err("This agent belongs to an unfinished account change. Retry or choose another account for that change.".into());
        }
    }
    Ok(())
}

pub(crate) fn guard_new_default(app: &AppHandle, id: &str) -> Result<(), String> {
    if read(app)?.is_some_and(|op| {
        op.phase != "complete" && (op.change_default || op.source.as_deref() == Some(id))
    }) {
        return Err(
            "Finish the current account change before creating an agent on the default account"
                .into(),
        );
    }
    Ok(())
}

pub(crate) fn guard_removal(
    app: &AppHandle,
    id: &str,
    group_id: Option<&str>,
) -> Result<(), String> {
    if read(app)?.is_some_and(|op| {
        op.phase != "complete"
            && group_id != Some(op.id.as_str())
            && (op.source.as_deref() == Some(id) || op.destination.as_deref() == Some(id))
    }) {
        return Err("Finish the current account change before removing this account".into());
    }
    Ok(())
}

pub(crate) fn guard_spawn(
    app: &AppHandle,
    record: &super::ManagedAgentRecord,
) -> Result<(), String> {
    if let Some(op) = read(app)?.filter(|op| op.phase != "complete") {
        if let Some(member) = op
            .members
            .iter()
            .find(|member| member.child.pubkey == record.pubkey && !member.mode_only)
        {
            if account_switch::read(app, &record.pubkey)?
                .is_none_or(|child| child.id != member.child.id)
            {
                return Err("This account change has not prepared the agent yet".into());
            }
        }
    }
    Ok(())
}

pub(crate) fn allows_staged_default(
    app: &AppHandle,
    record: &super::ManagedAgentRecord,
) -> Result<bool, String> {
    Ok(read(app)?.is_some_and(|op| {
        op.phase != "complete"
            && op.destination == record.subscription_account_id
            && op.members.iter().any(|member| {
                member.child.pubkey == record.pubkey
                    && member.child.generation == record.subscription_account_generation
            })
    }))
}

#[tauri::command]
pub(crate) fn get_claude_account_change_status(app: AppHandle) -> Result<Option<Status>, String> {
    // A failed journal write can outlive its worker. Preserve the disk intent
    // for recovery, but never strand the UI in an unfinishable busy state.
    Ok(read(&app)?.map(|operation| visible_status(&operation, worker_lock().try_lock().is_err())))
}

fn visible_status(operation: &Operation, worker_active: bool) -> Status {
    let mut status = operation.status();
    if status.phase != "complete" && !worker_active && status.error.is_none() {
        status.phase = "paused".into();
        status.error =
            Some("Account change interrupted. Retry to continue from saved progress.".into());
    }
    status
}

#[tauri::command]
pub(crate) async fn preview_claude_account_change(
    request: ChangeRequest,
    app: AppHandle,
) -> Result<Preview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let operation = plan::capture(&app, request, None)?;
        Ok(Preview {
            fingerprint: plan::fingerprint(&operation)?,
            operation: operation.status(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn apply_claude_account_change(
    request: ChangeRequest,
    fingerprint: String,
    app: AppHandle,
) -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _worker = worker_lock()
            .lock()
            .map_err(|_| "Account change worker unavailable")?;
        let operation = plan::capture(&app, request, Some(&fingerprint))?;
        execute::run(&app, operation)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn retry_claude_account_change(
    id: String,
    destination: Option<String>,
    app: AppHandle,
) -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _worker = worker_lock()
            .lock()
            .map_err(|_| "Account change worker unavailable")?;
        let mut operation = read(&app)?.ok_or("Account change not found")?;
        if operation.id != id {
            return Err("The account change was replaced; refresh its status".into());
        }
        if operation.phase == "complete" {
            return Err("This account change has already completed".into());
        }
        if let Some(destination) = destination {
            plan::retarget(&app, &mut operation, &destination)?;
        }
        operation.phase = "applying".into();
        operation.error = None;
        for member in &mut operation.members {
            member.error = None;
        }
        save(&app, &operation)?;
        execute::run(&app, operation)
    })
    .await
    .map_err(|e| e.to_string())?
}

pub(crate) fn recover(app: &AppHandle) -> Result<(), String> {
    let _worker = worker_lock()
        .lock()
        .map_err(|_| "Account change worker unavailable")?;
    if let Some(operation) = read(app)?.filter(|op| op.phase != "complete" && op.error.is_none()) {
        execute::run(app, operation)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
