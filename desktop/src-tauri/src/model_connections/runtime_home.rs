//! Lifecycle of generated connection homes; hosted and legacy homes are untouched.
use super::{ModelConnection, CONNECTION_ENV};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tauri::Manager;

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Selection {
    connection: String,
    base_url: String,
    model: String,
}

fn read(path: &Path) -> Result<Selection, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path.join("connection.json"))
        .map_err(|_| "Cannot read generated connection home")?
        .take(16385)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read generated connection home")?;
    serde_json::from_slice(&bytes).map_err(|_| "Invalid generated connection home".into())
}

pub(super) fn prepare(
    root: &Path,
    agent: &str,
    row: &ModelConnection,
    model: &str,
) -> Result<PathBuf, String> {
    if agent.len() != 64 || !agent.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid agent identity for model connection".into());
    }
    let name = format!("codex-{}", hex::encode(Sha256::digest(row.id.as_bytes())));
    let home = root.join(agent).join(name);
    let selection = Selection {
        connection: row.id.clone(),
        base_url: row.base_url.clone(),
        model: model.into(),
    };
    if home.exists() && read(&home).ok().as_ref() != Some(&selection) {
        std::fs::remove_dir_all(&home).map_err(|_| "Cannot regenerate Codex connection home")?;
    }
    std::fs::create_dir_all(&home).map_err(|_| "Cannot create Codex connection home")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot protect Codex connection home")?;
    }
    super::runtime::write_private(
        &home.join("connection.json"),
        &serde_json::to_vec(&selection).map_err(|e| e.to_string())?,
    )?;
    Ok(home)
}

fn homes(root: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let agents = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(_) => return Err("Cannot inspect generated connection homes".into()),
    };
    let mut homes = Vec::new();
    for entry in agents {
        let entry = entry.map_err(|_| "Cannot inspect generated connection homes")?;
        let agent = entry.file_name().to_string_lossy().into_owned();
        if agent.len() != 64
            || !agent.bytes().all(|b| b.is_ascii_hexdigit())
            || !entry
                .file_type()
                .map_err(|_| "Cannot inspect generated connection homes")?
                .is_dir()
        {
            continue;
        }
        for child in std::fs::read_dir(entry.path())
            .map_err(|_| "Cannot inspect generated connection homes")?
        {
            let child = child.map_err(|_| "Cannot inspect generated connection homes")?;
            let name = child.file_name().to_string_lossy().into_owned();
            if name
                .strip_prefix("codex-")
                .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
                && child
                    .file_type()
                    .map_err(|_| "Cannot inspect generated connection homes")?
                    .is_dir()
            {
                homes.push((agent.clone(), child.path()));
            }
        }
    }
    Ok(homes)
}

pub(super) fn remove_connection(root: &Path, id: &str) -> Result<(), String> {
    for (_, home) in homes(root)? {
        if read(&home).map_or(true, |selection| selection.connection == id) {
            std::fs::remove_dir_all(home).map_err(|_| "Cannot remove generated connection home")?;
        }
    }
    Ok(())
}

fn retain(root: &Path, selected: &BTreeMap<String, (String, String)>) -> Result<(), String> {
    for (agent, home) in homes(root)? {
        let keep = read(&home).ok().is_some_and(|selection| {
            selected.get(&agent).is_some_and(|(connection, model)| {
                *connection == selection.connection && *model == selection.model
            })
        });
        if !keep {
            std::fs::remove_dir_all(home).map_err(|_| "Cannot remove obsolete connection home")?;
        }
    }
    Ok(())
}

pub(crate) fn reconcile<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    records: &[crate::managed_agents::ManagedAgentRecord],
) -> Result<(), String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("model-connection-runtimes");
    if !root.exists() {
        return Ok(());
    }
    let definitions = records
        .iter()
        .filter(|record| record.pubkey.is_empty())
        .filter_map(|record| record.to_definition_view())
        .collect::<Vec<_>>();
    let global = crate::managed_agents::load_global_agent_config(app)?;
    let mut selected = BTreeMap::new();
    for record in records.iter().filter(|record| !record.pubkey.is_empty()) {
        let Ok(descriptor) = crate::managed_agents::resolve_effective_harness_descriptor(
            record,
            &definitions,
            &global,
        ) else {
            continue;
        };
        if crate::managed_agents::known_acp_runtime(&descriptor.command).map(|meta| meta.id)
            != Some("codex")
        {
            continue;
        }
        let Some(connection) = descriptor
            .env
            .get(CONNECTION_ENV)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let (model, _) =
            crate::managed_agents::resolve_effective_model_provider(record, &definitions, &global);
        if let Some(model) = model {
            selected.insert(record.pubkey.clone(), (connection.clone(), model));
        }
    }
    retain(&root, &selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row() -> ModelConnection {
        serde_json::from_value(serde_json::json!({"id":"connection","name":"source","base_url":"http://localhost/v1","models":["model"]})).unwrap()
    }
    #[test]
    fn deleting_agent_or_switching_to_hosted_removes_only_its_generated_home() {
        let root = tempfile::tempdir().unwrap();
        let row = row();
        let agent = "a".repeat(64);
        let other = "b".repeat(64);
        let first = prepare(root.path(), &agent, &row, "model").unwrap();
        let second = prepare(root.path(), &other, &row, "model").unwrap();
        let retained = BTreeMap::from([(other.clone(), (row.id.clone(), "model".into()))]);
        retain(root.path(), &retained).unwrap();
        assert!(!first.exists());
        assert!(second.exists());
        retain(root.path(), &BTreeMap::new()).unwrap();
        assert!(!second.exists());
    }
    #[test]
    fn connection_removal_and_edits_invalidate_generated_config_and_history() {
        let root = tempfile::tempdir().unwrap();
        let mut row = row();
        let agent = "a".repeat(64);
        let home = prepare(root.path(), &agent, &row, "model").unwrap();
        std::fs::write(home.join("history.jsonl"), "old").unwrap();
        row.base_url = "http://localhost:8001/v1".into();
        prepare(root.path(), &agent, &row, "model").unwrap();
        assert!(!home.join("history.jsonl").exists());
        std::fs::write(home.join("history.jsonl"), "old").unwrap();
        prepare(root.path(), &agent, &row, "other-model").unwrap();
        assert!(!home.join("history.jsonl").exists());
        remove_connection(root.path(), &row.id).unwrap();
        assert!(!home.exists());
    }
}
