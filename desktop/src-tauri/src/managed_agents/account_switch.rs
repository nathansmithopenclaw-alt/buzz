//! Durable intent for a single agent's token-account handoff on this Mac.
use super::ManagedAgentRuntimeKey;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, process::Command};
use tauri::{AppHandle, Manager};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Runtime {
    pub key: ManagedAgentRuntimeKey,
    pub nonce: String,
    pub control_dir: PathBuf,
    pub paused: bool,
    pub run: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Operation {
    pub id: String,
    pub pubkey: String,
    pub account_id: String,
    pub generation: u64,
    pub phase: String,
    pub error: Option<String>,
    pub runtimes: Vec<Runtime>,
}

fn directory(app: &AppHandle) -> Result<PathBuf, String> {
    let path = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("agents")
        .join("account-switches");
    super::subscription_binding::private_runtime_dir(&path)?;
    Ok(path)
}

fn path(app: &AppHandle, pubkey: &str) -> Result<PathBuf, String> {
    if pubkey.len() != 64 || !pubkey.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid agent identity".into());
    }
    Ok(directory(app)?.join(format!("{}.json", pubkey.to_ascii_lowercase())))
}

pub(crate) fn read(app: &AppHandle, pubkey: &str) -> Result<Option<Operation>, String> {
    let path = path(app, pubkey)?;
    let runtime_root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("agents")
        .join("subscription-runtime");
    read_at(&path, pubkey, &runtime_root)
}
fn read_at(
    path: &std::path::Path,
    pubkey: &str,
    runtime_root: &std::path::Path,
) -> Result<Option<Operation>, String> {
    let Some(raw) = super::subscription_binding::read_control_file(path, 65536)? else {
        return Ok(None);
    };
    let operation: Operation =
        serde_json::from_str(&raw).map_err(|_| "Invalid account switch journal")?;
    if operation.pubkey != pubkey
        || operation.id.len() != 32
        || !operation.id.bytes().all(|c| c.is_ascii_hexdigit())
        || !["pausing", "paused", "restarting", "complete"].contains(&operation.phase.as_str())
        || operation.runtimes.iter().any(|r| r.key.pubkey != pubkey)
    {
        return Err("Invalid account switch journal identity".into());
    }
    for runtime in &operation.runtimes {
        let key = ManagedAgentRuntimeKey::new(runtime.key.pubkey.clone(), &runtime.key.relay_url)?;
        if key != runtime.key {
            return Err("Noncanonical account switch runtime".into());
        }
        if runtime.nonce.is_empty() && runtime.control_dir.as_os_str().is_empty() && runtime.paused
        {
            continue;
        }
        if runtime.nonce.len() != 32
            || !runtime.nonce.bytes().all(|c| c.is_ascii_hexdigit())
            || runtime.control_dir != runtime_root.join(key.runtime_id()).join(&runtime.nonce)
        {
            return Err("Invalid account switch control identity".into());
        }
    }
    Ok(Some(operation))
}

pub(crate) fn save(app: &AppHandle, operation: &Operation) -> Result<(), String> {
    let path = path(app, &operation.pubkey)?;
    save_at(&path, operation)
}
fn save_at(path: &std::path::Path, operation: &Operation) -> Result<(), String> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| !m.is_file() || m.file_type().is_symlink()) {
        return Err("Invalid account switch journal path".into());
    }
    super::atomic_write_json_restricted(
        path,
        &serde_json::to_vec(operation).map_err(|e| e.to_string())?,
    )?;
    std::fs::File::open(path.parent().ok_or("Missing journal directory")?)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())
}

/// Enforced at the common spawn boundary, including launch restore and reconciliation.
pub(crate) fn check_spawn(
    app: &AppHandle,
    record: &super::ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
) -> Result<(), String> {
    if let Some(op) = read(app, &record.pubkey)? {
        admit_spawn(
            &op,
            record.subscription_account_id.as_deref(),
            record.subscription_account_generation,
            key,
        )?;
    }
    Ok(())
}
fn admit_spawn(
    op: &Operation,
    selected: Option<&str>,
    generation: u64,
    key: &ManagedAgentRuntimeKey,
) -> Result<(), String> {
    if op.phase == "pausing"
        || op.phase == "paused"
        || op.error.is_some()
        || selected != Some(&op.account_id)
        || generation != op.generation
    {
        return Err("Account switch paused; retry or choose another account".into());
    }
    if !op.runtimes.iter().any(|r| &r.key == key && r.run) {
        return Err(
            "This runtime was stopped when the account changed; start it explicitly".into(),
        );
    }
    Ok(())
}

pub(crate) fn configure(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
    command: &mut Command,
) -> Result<(), String> {
    command
        .env_remove("BUZZ_ACP_ACCOUNT_SWITCH_ID")
        .env_remove("BUZZ_ACP_ACCOUNT_SWITCH_RESUME");
    if let Some(op) = read(app, &key.pubkey)? {
        command.env("BUZZ_ACP_ACCOUNT_SWITCH_ID", &op.id).env(
            "BUZZ_ACP_ACCOUNT_SWITCH_RESUME",
            if op.phase == "complete" { "1" } else { "0" },
        );
    }
    Ok(())
}

/// An explicit Start/Stop changes intent; automatic reconciliation cannot change it.
pub(crate) fn set_running(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
    run: bool,
) -> Result<(), String> {
    if let Some(mut op) = read(app, &key.pubkey)? {
        if run && op.phase != "complete" {
            return Err("Finish or retry the account switch first".into());
        }
        if let Some(runtime) = op.runtimes.iter_mut().find(|r| &r.key == key) {
            runtime.run = run;
        } else if run {
            op.runtimes.push(Runtime {
                key: key.clone(),
                nonce: String::new(),
                control_dir: PathBuf::new(),
                paused: true,
                run,
            });
        }
        save(app, &op)?;
    }
    Ok(())
}

/// Non-secret progress survives closing the picker and restarting Desktop.
#[derive(Serialize)]
pub struct Status {
    id: String,
    account_id: String,
    phase: String,
    error: Option<String>,
}

#[tauri::command]
pub fn agent_account_switch_status(
    pubkey: String,
    app: AppHandle,
) -> Result<Option<Status>, String> {
    Ok(read(&app, &pubkey)?.map(|op| Status {
        id: op.id,
        account_id: op.account_id,
        phase: op.phase,
        error: op.error,
    }))
}

pub(crate) fn recover(app: &AppHandle) -> Result<(), String> {
    let pubkeys: Vec<_> = super::load_managed_agents(app)?
        .into_iter()
        .map(|r| r.pubkey)
        .collect();
    for pubkey in pubkeys {
        let operation = match read(app, &pubkey) {
            Ok(operation) => operation,
            Err(error) => {
                eprintln!("buzz-desktop: invalid account switch journal for {pubkey}: {error}");
                continue;
            }
        };
        if let Some(op) = operation {
            if op.phase != "complete" && op.error.is_none() {
                if let Err(error) = super::subscription_binding::switch_account(
                    pubkey,
                    Some(op.account_id),
                    app.clone(),
                    None,
                ) {
                    eprintln!("buzz-desktop: account switch recovery paused: {error}");
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &std::path::Path) -> Operation {
        let key = ManagedAgentRuntimeKey::new("a".repeat(64), "wss://relay.example").unwrap();
        let nonce = "b".repeat(32);
        Operation {
            id: "c".repeat(32),
            pubkey: key.pubkey.clone(),
            account_id: "destination".into(),
            generation: 7,
            phase: "pausing".into(),
            error: None,
            runtimes: vec![Runtime {
                control_dir: root.join(key.runtime_id()).join(&nonce),
                key,
                nonce,
                paused: false,
                run: true,
            }],
        }
    }
    #[test]
    fn disk_recovery_retains_every_phase_destination_and_stopped_intent() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("operation.json");
        let mut op = fixture(root.path());
        for phase in ["pausing", "paused", "restarting", "complete"] {
            op.phase = phase.into();
            op.runtimes[0].run = false;
            save_at(&path, &op).unwrap();
            let restored = read_at(&path, &op.pubkey, root.path()).unwrap().unwrap();
            assert_eq!(restored.phase, phase);
            assert_eq!(restored.account_id, "destination");
            assert_eq!(restored.generation, 7);
            assert!(!restored.runtimes[0].run);
            assert!(
                admit_spawn(&restored, Some("destination"), 7, &restored.runtimes[0].key).is_err()
            );
        }
        op.runtimes[0].control_dir = root.path().join("unrelated");
        save_at(&path, &op).unwrap();
        assert!(read_at(&path, &op.pubkey, root.path()).is_err());
    }
    #[test]
    fn spawn_requires_committed_destination_generation_and_exact_running_pair() {
        let root = tempfile::tempdir().unwrap();
        let mut op = fixture(root.path());
        let key = op.runtimes[0].key.clone();
        assert!(admit_spawn(&op, Some("destination"), 7, &key).is_err());
        op.phase = "restarting".into();
        assert!(admit_spawn(&op, Some("destination"), 7, &key).is_ok());
        assert!(admit_spawn(&op, Some("old-payer"), 7, &key).is_err());
        assert!(admit_spawn(&op, Some("destination"), 6, &key).is_err());
        let other = ManagedAgentRuntimeKey::new(&op.pubkey, "wss://other.example").unwrap();
        assert!(admit_spawn(&op, Some("destination"), 7, &other).is_err());
        op.error = Some("Destination unavailable".into());
        assert!(admit_spawn(&op, Some("destination"), 7, &key).is_err());
    }
}
