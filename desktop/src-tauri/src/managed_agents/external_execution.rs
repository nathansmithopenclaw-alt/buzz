//! Host-local execution ownership for agents operated by another application.
//! This binding is separate from portable personas and identity credentials.
//! Presence and model settings remain the external application's authority.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::ManagedAgentRuntimeKey;

/// The only supported external execution target. URLs are derived, never saved.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExternalExecution {
    CodexDesktop { thread_id: String },
}

impl ExternalExecution {
    pub(crate) fn task_url(&self) -> Result<String, String> {
        match self {
            Self::CodexDesktop { thread_id } => {
                let id = uuid::Uuid::parse_str(thread_id)
                    .map_err(|_| "External execution has an invalid Codex task ID")?;
                Ok(format!("codex://threads/{id}"))
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    version: u8,
    pubkey: String,
    relay_url: String,
    target: ExternalExecution,
}

pub(crate) fn load_from(
    agents_dir: &Path,
    key: &ManagedAgentRuntimeKey,
) -> Result<Option<ExternalExecution>, String> {
    if key.pubkey.len() != 64 || !key.pubkey.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("External execution requires a valid agent identity".into());
    }
    let file = agents_dir
        .join("external-execution")
        .join(format!("{}.json", key.pubkey.to_ascii_lowercase()));
    let input = match File::open(file) {
        Ok(input) => input,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Cannot read this agent's external execution binding".into()),
    };
    let mut bytes = Vec::new();
    input
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read this agent's external execution binding")?;
    if bytes.len() > 4096 {
        return Err("External execution binding exceeds its size limit".into());
    }
    let binding: Binding = serde_json::from_slice(&bytes)
        .map_err(|_| "This agent's external execution binding needs repair")?;
    if binding.version != 1 || !binding.pubkey.eq_ignore_ascii_case(&key.pubkey) {
        return Err("External execution binding version or identity does not match".into());
    }
    binding.target.task_url()?;
    let bound_key = ManagedAgentRuntimeKey::new(binding.pubkey, &binding.relay_url)?;
    if bound_key.relay_url != key.relay_url {
        return Ok(None);
    }
    Ok(Some(binding.target))
}

/// Read the binding for the exact runtime pair, without using a legacy relay pin.
pub(crate) fn load(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
) -> Result<Option<ExternalExecution>, String> {
    load_from(&super::managed_agents_base_dir(app)?, key)
}

/// Common spawn fence. Missing or invalid external data must never allow fallback.
pub(crate) fn require_local(app: &AppHandle, key: &ManagedAgentRuntimeKey) -> Result<(), String> {
    require_local_from(&super::managed_agents_base_dir(app)?, key)
}

pub(crate) fn require_local_from(
    agents_dir: &Path,
    key: &ManagedAgentRuntimeKey,
) -> Result<(), String> {
    if load_from(agents_dir, key)?.is_some() {
        return Err(
            "This agent runs in Codex Desktop. Open its task there to manage execution.".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(value: serde_json::Value) -> (tempfile::TempDir, ManagedAgentRuntimeKey) {
        let root = tempfile::tempdir().expect("fixture directory");
        let key =
            ManagedAgentRuntimeKey::new("aa".repeat(32), "wss://one.example").expect("fixture key");
        let dir = root.path().join("external-execution");
        std::fs::create_dir(&dir).expect("binding directory");
        std::fs::write(dir.join(format!("{}.json", key.pubkey)), value.to_string())
            .expect("binding");
        (root, key)
    }

    fn binding() -> serde_json::Value {
        json!({"version":1,"pubkey":"aa".repeat(32),"relay_url":"wss://one.example/",
            "target":{"type":"codex_desktop","thread_id":"01a08c97-4b41-7000-bbd6-b58aa4b34eef"}})
    }

    #[test]
    fn external_binding_fences_local_spawn_for_only_its_exact_pair() {
        let (dir, key) = fixture(binding());
        assert!(require_local_from(dir.path(), &key).is_err());
        let target = load_from(dir.path(), &key)
            .expect("binding")
            .expect("external");
        assert_eq!(
            target.task_url().expect("url"),
            "codex://threads/01a08c97-4b41-7000-bbd6-b58aa4b34eef"
        );
        let other_relay =
            ManagedAgentRuntimeKey::new(key.pubkey.clone(), "wss://two.example").expect("key");
        assert!(require_local_from(dir.path(), &other_relay).is_ok());
        let other_agent =
            ManagedAgentRuntimeKey::new("bb".repeat(32), &key.relay_url).expect("key");
        assert!(require_local_from(dir.path(), &other_agent).is_ok());
    }

    #[test]
    fn corrupt_unknown_or_retargeted_binding_never_falls_back_to_local() {
        for edit in [
            json!({"version":2}),
            json!({"pubkey":"bb".repeat(32)}),
            json!({"target":{"type":"codex_desktop","thread_id":"../../another-task"}}),
            json!({"target":{"type":"shell","command":"untrusted"}}),
            json!({"extra":"ignored?"}),
        ] {
            let mut value = binding();
            for (key, content) in edit.as_object().expect("edit") {
                value[key] = content.clone();
            }
            let (dir, key) = fixture(value);
            assert!(require_local_from(dir.path(), &key).is_err());
        }
    }

    #[test]
    fn absent_binding_allows_existing_local_agents_and_oversized_binding_is_refused() {
        let dir = tempfile::tempdir().expect("directory");
        let key = ManagedAgentRuntimeKey::new("aa".repeat(32), "wss://one.example").expect("key");
        assert!(require_local_from(dir.path(), &key).is_ok());
        let (dir, key) = fixture(serde_json::Value::String("x".repeat(4097)));
        assert!(require_local_from(dir.path(), &key).is_err());
    }
}
