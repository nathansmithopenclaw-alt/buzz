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
    prepare_for(root, agent, row, model, "codex")
}

pub(super) fn prepare_for(
    root: &Path,
    agent: &str,
    row: &ModelConnection,
    model: &str,
    runtime: &str,
) -> Result<PathBuf, String> {
    if !matches!(runtime, "codex" | "goose") {
        return Err("Unsupported native connection home".into());
    }
    if agent.len() != 64 || !agent.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid agent identity for model connection".into());
    }
    let name = format!(
        "{runtime}-{}",
        hex::encode(Sha256::digest(row.id.as_bytes()))
    );
    let home = root.join(agent).join(name);
    let selection = Selection {
        connection: row.id.clone(),
        base_url: row.base_url.clone(),
        model: model.into(),
    };
    if home.exists() && read(&home).ok().as_ref() != Some(&selection) {
        std::fs::remove_dir_all(&home).map_err(|_| "Cannot regenerate native connection home")?;
    }
    std::fs::create_dir_all(&home).map_err(|_| "Cannot create native connection home")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot protect native connection home")?;
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
                .or_else(|| name.strip_prefix("goose-"))
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

fn retain(
    root: &Path,
    selected: &BTreeMap<String, (String, String, String)>,
) -> Result<(), String> {
    for (agent, home) in homes(root)? {
        let keep = read(&home).ok().is_some_and(|selection| {
            selected
                .get(&agent)
                .is_some_and(|(runtime, connection, model)| {
                    home.file_name().is_some_and(|name| {
                        name.to_string_lossy().starts_with(&format!("{runtime}-"))
                    }) && *connection == selection.connection
                        && *model == selection.model
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
    previous: &[crate::managed_agents::ManagedAgentRecord],
) -> Result<(), String> {
    let global = crate::managed_agents::load_global_agent_config(app)?;
    reconcile_for_records(app, records, &global, Some(previous))
}

pub(crate) fn reconcile_with_global<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    records: &[crate::managed_agents::ManagedAgentRecord],
    global: &crate::managed_agents::GlobalAgentConfig,
) -> Result<(), String> {
    reconcile_for_records(app, records, global, None)
}

fn reconcile_for_records<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    records: &[crate::managed_agents::ManagedAgentRecord],
    global: &crate::managed_agents::GlobalAgentConfig,
    previous: Option<&[crate::managed_agents::ManagedAgentRecord]>,
) -> Result<(), String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("model-connection-runtimes");
    reconcile_at(
        &root,
        records,
        global,
        previous,
        super::native_goose::runtime_reason,
    )
}

fn reconcile_at(
    root: &Path,
    records: &[crate::managed_agents::ManagedAgentRecord],
    global: &crate::managed_agents::GlobalAgentConfig,
    previous: Option<&[crate::managed_agents::ManagedAgentRecord]>,
    mut runtime_reason: impl FnMut() -> Option<String>,
) -> Result<(), String> {
    let definitions = records
        .iter()
        .filter(|record| record.pubkey.is_empty())
        .filter_map(|record| record.to_definition_view())
        .collect::<Vec<_>>();
    let connection_path = root
        .parent()
        .ok_or("Invalid runtime directory")?
        .join("model-connections.json");
    let mut checked = std::collections::BTreeSet::new();
    let mut validate_goose = |id: &str| {
        if checked.insert(id.to_owned()) {
            validate_goose_source(&connection_path, id, &mut runtime_reason)?;
        }
        Ok::<_, String>(())
    };
    if previous.is_none() && global.preferred_runtime.as_deref() == Some("goose") {
        if let Some(id) = global
            .env_vars
            .get(CONNECTION_ENV)
            .filter(|id| !id.is_empty())
        {
            validate_goose(id)?;
        }
    }
    let mut selected = BTreeMap::new();
    for record in records {
        let Ok(descriptor) = crate::managed_agents::resolve_effective_harness_descriptor(
            record,
            &definitions,
            global,
        ) else {
            continue;
        };
        let Some(runtime) =
            crate::managed_agents::known_acp_runtime(&descriptor.command).map(|meta| meta.id)
        else {
            continue;
        };
        if !matches!(runtime, "codex" | "goose") {
            continue;
        }
        let Some(connection) = descriptor
            .env
            .get(CONNECTION_ENV)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        // An incompatible existing selection must not block another agent's
        // save. Global saves validate all inherited selections; store saves
        // validate changed/new records (including changed definitions).
        let changed = previous.is_none_or(|old| !old.contains(record));
        if runtime == "goose" && changed {
            validate_goose(connection)?;
        }
        if record.pubkey.is_empty() {
            continue;
        }
        let (model, _) =
            crate::managed_agents::resolve_effective_model_provider(record, &definitions, global);
        if let Some(model) = model {
            selected.insert(
                record.pubkey.clone(),
                (runtime.into(), connection.clone(), model),
            );
        }
    }
    retain(root, &selected)
}

fn validate_goose_source(
    path: &Path,
    id: &str,
    runtime_reason: impl FnOnce() -> Option<String>,
) -> Result<(), String> {
    let row = super::load_file(path)?
        .into_iter()
        .find(|row| row.id == id)
        .ok_or_else(|| format!("Saved connection {id} is unavailable. Choose another model."))?;
    super::native_goose::validate_key(row.has_key || row.secret_ref.is_some())?;
    match runtime_reason() {
        Some(reason) => Err(reason),
        None => Ok(()),
    }
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
        let retained = BTreeMap::from([(
            other.clone(),
            ("codex".into(), row.id.clone(), "model".into()),
        )]);
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
    #[test]
    fn goose_save_admission_refuses_keyed_missing_and_unqualified_runtime() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("model-connections.json");
        let mut source = row();
        for keyed in [false, true] {
            source.has_key = keyed;
            super::super::persist_file(&file, &[source.clone()]).unwrap();
            let result = validate_goose_source(&file, "connection", || None);
            if keyed {
                assert_eq!(
                    result.unwrap_err(),
                    super::super::native_goose::KEYED_REASON
                );
            } else {
                result.unwrap();
            }
        }
        source.has_key = false;
        super::super::persist_file(&file, &[source.clone()]).unwrap();
        assert!(validate_goose_source(&file, "missing", || None)
            .unwrap_err()
            .contains("missing"));
        assert_eq!(
            validate_goose_source(&file, "connection", || Some(
                "qualified version mismatch".into()
            ))
            .unwrap_err(),
            "qualified version mismatch"
        );
    }

    #[test]
    fn switching_native_harness_removes_old_home_and_connection_removal_covers_both() {
        let root = tempfile::tempdir().unwrap();
        let source = row();
        let agent = "a".repeat(64);
        let codex = prepare(root.path(), &agent, &source, "model").unwrap();
        let goose = prepare_for(root.path(), &agent, &source, "model", "goose").unwrap();
        retain(
            root.path(),
            &BTreeMap::from([(
                agent.clone(),
                ("goose".into(), source.id.clone(), "model".into()),
            )]),
        )
        .unwrap();
        assert!(!codex.exists());
        assert!(goose.exists());
        prepare(root.path(), &agent, &source, "model").unwrap();
        remove_connection(root.path(), &source.id).unwrap();
        assert!(!codex.exists() && !goose.exists());
    }
    #[test]
    fn central_save_validates_definition_instance_and_global_before_any_home_exists() {
        let root = tempfile::tempdir().unwrap();
        let homes = root.path().join("model-connection-runtimes");
        let mut source = row();
        let definition: crate::managed_agents::AgentDefinition = serde_json::from_value(serde_json::json!({
            "id":"test", "display_name":"Test", "system_prompt":"Test", "runtime":"goose", "model":"model", "env_vars":{CONNECTION_ENV:"connection"}, "created_at":"", "updated_at":""
        })).unwrap();
        let record = definition.into_agent_record();
        let global = crate::managed_agents::GlobalAgentConfig::default();
        for keyed in [false, true] {
            source.has_key = keyed;
            super::super::persist_file(
                &root.path().join("model-connections.json"),
                &[source.clone()],
            )
            .unwrap();
            for instance in [false, true] {
                let mut candidate = record.clone();
                if instance {
                    candidate.pubkey = "a".repeat(64);
                }
                let result = reconcile_at(&homes, &[candidate], &global, None, || None);
                if keyed {
                    assert_eq!(
                        result.unwrap_err(),
                        super::super::native_goose::KEYED_REASON
                    );
                } else {
                    result.unwrap();
                }
            }
            let candidate = crate::managed_agents::GlobalAgentConfig {
                preferred_runtime: Some("goose".into()),
                model: Some("model".into()),
                env_vars: BTreeMap::from([(CONNECTION_ENV.into(), "connection".into())]),
                ..Default::default()
            };
            let result = reconcile_at(&homes, &[], &candidate, None, || None);
            if keyed {
                assert_eq!(
                    result.unwrap_err(),
                    super::super::native_goose::KEYED_REASON
                );
            } else {
                result.unwrap();
            }
        }
        let previous = vec![record.clone()];
        // The source is now keyed. An unchanged Goose record cannot prevent
        // a hosted record from being saved, but editing that Goose record must.
        let mut hosted = record.clone();
        hosted.slug = Some("hosted".into());
        hosted.runtime = Some("codex".into());
        hosted.env_vars.clear();
        reconcile_at(
            &homes,
            &[record.clone(), hosted],
            &global,
            Some(&previous),
            || None,
        )
        .unwrap();
        let mut changed = record;
        changed.name = "Changed Goose".into();
        assert_eq!(
            reconcile_at(&homes, &[changed], &global, Some(&previous), || None).unwrap_err(),
            super::super::native_goose::KEYED_REASON
        );
        assert!(!homes.exists());
    }
}
