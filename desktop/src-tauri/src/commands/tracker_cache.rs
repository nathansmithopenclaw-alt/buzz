use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

const CACHE_VERSION: u8 = 1;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TrackerFoldCacheKey {
    pub(crate) relay_url: String,
    pub(crate) workspace_id: String,
    pub(crate) managed_signer: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TrackerFoldCacheEntry {
    version: u8,
    relay_url: String,
    workspace_id: String,
    managed_signer: String,
    pub(crate) cursor: Option<u64>,
    pub(crate) events: BTreeMap<String, Value>,
    pub(crate) task_events: BTreeMap<String, Value>,
    pub(crate) membership_event: Value,
}

/// Rust-owned raw event history backing cold, resumed, and live Dispatch folds.
///
/// The persisted representation is scoped by canonical relay URL, workspace,
/// and signer. The renderer sees only a timestamp cursor and folded output; it
/// never owns or mutates the raw history used by the SDK fold.
#[derive(Default)]
pub(crate) struct TrackerFoldCache {
    entries: Mutex<HashMap<TrackerFoldCacheKey, TrackerFoldCacheEntry>>,
}

fn raw_event_key(raw: &Value) -> String {
    raw.get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(|id| format!("id:{id}"))
        .unwrap_or_else(|| format!("raw:{raw}"))
}

fn index_raw_events(events: Vec<Value>) -> BTreeMap<String, Value> {
    events
        .into_iter()
        .map(|event| (raw_event_key(&event), event))
        .collect()
}

fn merge_raw_events(target: &mut BTreeMap<String, Value>, events: Vec<Value>) {
    for event in events {
        target.insert(raw_event_key(&event), event);
    }
}

impl TrackerFoldCacheEntry {
    pub(crate) fn new(
        key: &TrackerFoldCacheKey,
        cursor: Option<u64>,
        events: Vec<Value>,
        task_events: Vec<Value>,
        membership_event: Value,
    ) -> Self {
        Self {
            version: CACHE_VERSION,
            relay_url: key.relay_url.clone(),
            workspace_id: key.workspace_id.clone(),
            managed_signer: key.managed_signer.clone(),
            cursor,
            events: index_raw_events(events),
            task_events: index_raw_events(task_events),
            membership_event,
        }
    }

    pub(crate) fn merge(
        &mut self,
        events: Vec<Value>,
        task_events: Vec<Value>,
        membership_event: Option<Value>,
    ) {
        merge_raw_events(&mut self.events, events);
        merge_raw_events(&mut self.task_events, task_events);
        if let Some(membership_event) = membership_event {
            self.membership_event = membership_event;
        }
    }

    fn belongs_to(&self, key: &TrackerFoldCacheKey) -> bool {
        self.version == CACHE_VERSION
            && self.relay_url == key.relay_url
            && self.workspace_id == key.workspace_id
            && self.managed_signer == key.managed_signer
    }
}

impl TrackerFoldCache {
    pub(crate) fn clear(&self) {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    pub(crate) fn seed(
        &self,
        key: TrackerFoldCacheKey,
        entry: TrackerFoldCacheEntry,
    ) -> Result<(), String> {
        self.entries
            .lock()
            .map_err(|error| error.to_string())?
            .insert(key, entry);
        Ok(())
    }

    pub(crate) fn get(
        &self,
        key: &TrackerFoldCacheKey,
    ) -> Result<Option<TrackerFoldCacheEntry>, String> {
        Ok(self
            .entries
            .lock()
            .map_err(|error| error.to_string())?
            .get(key)
            .cloned())
    }

    pub(crate) fn merge(
        &self,
        key: &TrackerFoldCacheKey,
        events: Vec<Value>,
        task_events: Vec<Value>,
        membership_event: Option<Value>,
    ) -> Result<TrackerFoldCacheEntry, String> {
        let entries = self.entries.lock().map_err(|error| error.to_string())?;
        let mut entry = entries.get(key).cloned().ok_or_else(|| {
            "incremental Tracker fold has no persisted history; refresh the workspace".to_string()
        })?;
        entry.merge(events, task_events, membership_event);
        Ok(entry)
    }
}

pub(crate) fn cache_key(
    relay_url: &str,
    workspace_id: &str,
    managed_signer: &str,
) -> Result<TrackerFoldCacheKey, String> {
    let relay_url = buzz_core_pkg::relay::normalize_relay_url(relay_url)
        .map_err(|error| format!("invalid Tracker relay URL: {error}"))?;
    let workspace_id = workspace_id
        .parse::<Uuid>()
        .map_err(|error| format!("invalid Tracker workspace UUID: {error}"))?
        .to_string();
    let managed_signer = nostr::PublicKey::parse(managed_signer)
        .map_err(|error| format!("invalid Tracker managed signer: {error}"))?
        .to_hex();
    Ok(TrackerFoldCacheKey {
        relay_url,
        workspace_id,
        managed_signer,
    })
}

fn cache_path(root: &Path, key: &TrackerFoldCacheKey) -> PathBuf {
    let relay_hash = hex::encode(Sha256::digest(key.relay_url.as_bytes()));
    root.join("dispatch")
        .join("tracker-fold-cache-v1")
        .join(format!(
            "{relay_hash}__{}__{}.json",
            key.workspace_id, key.managed_signer
        ))
}

pub(crate) fn persist_entry(
    root: &Path,
    key: &TrackerFoldCacheKey,
    entry: &TrackerFoldCacheEntry,
) -> Result<(), String> {
    let path = cache_path(root, key);
    let parent = path
        .parent()
        .ok_or_else(|| "Tracker cache path has no parent".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("create Tracker cache directory: {error}"))?;
    let payload = serde_json::to_vec(entry)
        .map_err(|error| format!("serialize Tracker fold cache: {error}"))?;
    crate::managed_agents::storage::atomic_write_json_restricted(&path, &payload)
}

pub(crate) fn load_entry(
    root: &Path,
    key: &TrackerFoldCacheKey,
) -> Result<Option<TrackerFoldCacheEntry>, String> {
    let path = cache_path(root, key);
    let payload = match fs::read(&path) {
        Ok(payload) => payload,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("read Tracker fold cache: {error}")),
    };
    let entry: TrackerFoldCacheEntry = serde_json::from_slice(&payload)
        .map_err(|error| format!("parse Tracker fold cache: {error}"))?;
    if !entry.belongs_to(key) {
        return Err("Tracker fold cache scope does not match the active workspace".to_string());
    }
    Ok(Some(entry))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    fn key(relay_url: &str) -> TrackerFoldCacheKey {
        cache_key(
            relay_url,
            &Uuid::nil().to_string(),
            &Keys::generate().public_key().to_hex(),
        )
        .expect("cache key")
    }

    #[test]
    fn persisted_entry_round_trips_only_for_its_exact_scope() {
        let root = tempfile::tempdir().expect("tempdir");
        let key = key("wss://relay.example");
        let entry = TrackerFoldCacheEntry::new(
            &key,
            Some(123),
            vec![serde_json::json!({"id": "event"})],
            Vec::new(),
            serde_json::json!({"id": "membership"}),
        );
        persist_entry(root.path(), &key, &entry).expect("persist cache");

        let restored = load_entry(root.path(), &key)
            .expect("load cache")
            .expect("stored entry");
        assert_eq!(restored.cursor, Some(123));
        assert_eq!(restored.events.len(), 1);

        let other_relay = TrackerFoldCacheKey {
            relay_url: "wss://other.example".to_string(),
            ..key
        };
        assert!(load_entry(root.path(), &other_relay)
            .expect("other scope")
            .is_none());
    }

    #[test]
    fn malformed_persisted_entry_fails_closed() {
        let root = tempfile::tempdir().expect("tempdir");
        let key = key("wss://relay.example");
        let path = cache_path(root.path(), &key);
        fs::create_dir_all(path.parent().expect("cache parent")).expect("cache dir");
        fs::write(path, b"not-json").expect("malformed cache");

        assert!(load_entry(root.path(), &key).is_err());
    }
}
