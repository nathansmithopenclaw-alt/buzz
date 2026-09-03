//! Restart-durable receipts for mention events already answered by this agent.
//!
//! The relay remains the source of truth. This local file is a bounded cache of
//! exact reply targets, keyed by channel, so a transient answered-state query
//! failure cannot turn a healthy harness restart into another model turn.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const WATERMARK_VERSION: u32 = 1;
const MAX_RECEIPTS_PER_CHANNEL: usize = 2_048;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChannelWatermark {
    /// Newest signed answer timestamp represented by `receipts`.
    answered_through: u64,
    /// Exact triggering event id -> signed answer timestamp.
    receipts: BTreeMap<String, u64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WatermarkFile {
    version: u32,
    agent_pubkey: String,
    relay_scope: String,
    channels: BTreeMap<String, ChannelWatermark>,
}

/// A validated, agent-and-relay-scoped answered-event receipt store.
#[derive(Debug)]
pub(crate) struct AnsweredWatermarkStore {
    path: PathBuf,
    state: WatermarkFile,
}

impl AnsweredWatermarkStore {
    pub(crate) fn open(
        explicit_path: Option<&Path>,
        agent_pubkey: &str,
        relay_url: &str,
    ) -> anyhow::Result<Self> {
        let relay_scope = relay_scope(relay_url);
        let path = match explicit_path {
            Some(path) => path.to_path_buf(),
            None => default_path(agent_pubkey, &relay_scope)?,
        };
        let state = match fs::read(&path) {
            Ok(raw) => {
                let state: WatermarkFile = serde_json::from_slice(&raw)
                    .map_err(|error| anyhow::anyhow!("malformed watermark file: {error}"))?;
                if state.version != WATERMARK_VERSION {
                    anyhow::bail!(
                        "unsupported watermark version {} (expected {WATERMARK_VERSION})",
                        state.version
                    );
                }
                if state.agent_pubkey != agent_pubkey || state.relay_scope != relay_scope {
                    anyhow::bail!("watermark identity or relay scope mismatch");
                }
                state
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => WatermarkFile {
                version: WATERMARK_VERSION,
                agent_pubkey: agent_pubkey.to_owned(),
                relay_scope,
                channels: BTreeMap::new(),
            },
            Err(error) => return Err(error.into()),
        };
        Ok(Self { path, state })
    }

    /// Return exact answered ids still inside the configured replay horizon.
    pub(crate) fn answered_ids_since(&self, since: u64) -> HashSet<String> {
        self.state
            .channels
            .values()
            .flat_map(|channel| channel.receipts.iter())
            .filter(|(_, answered_at)| **answered_at >= since)
            .map(|(event_id, _)| event_id.clone())
            .collect()
    }

    /// Merge relay-verified reply receipts and atomically persist the result.
    pub(crate) fn merge_and_persist(
        &mut self,
        receipts: impl IntoIterator<Item = (Uuid, String, u64)>,
        retain_since: u64,
    ) -> anyhow::Result<()> {
        for (channel_id, event_id, answered_at) in receipts {
            let channel = self
                .state
                .channels
                .entry(channel_id.to_string())
                .or_default();
            channel.answered_through = channel.answered_through.max(answered_at);
            channel
                .receipts
                .entry(event_id)
                .and_modify(|timestamp| *timestamp = (*timestamp).max(answered_at))
                .or_insert(answered_at);
        }
        self.state.channels.retain(|_, channel| {
            channel
                .receipts
                .retain(|_, answered_at| *answered_at >= retain_since);
            while channel.receipts.len() > MAX_RECEIPTS_PER_CHANNEL {
                let Some(oldest) = channel
                    .receipts
                    .iter()
                    .min_by_key(|(_, answered_at)| **answered_at)
                    .map(|(event_id, _)| event_id.clone())
                else {
                    break;
                };
                channel.receipts.remove(&oldest);
            }
            !channel.receipts.is_empty()
        });
        persist_atomic(&self.path, &self.state)
    }

    #[cfg(test)]
    fn path(&self) -> &Path {
        &self.path
    }
}

fn relay_scope(relay_url: &str) -> String {
    hex::encode(Sha256::digest(relay_url.trim_end_matches('/').as_bytes()))
}

fn default_path(agent_pubkey: &str, relay_scope: &str) -> anyhow::Result<PathBuf> {
    let base = dirs::data_local_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine local data directory"))?;
    Ok(base
        .join("buzz")
        .join("acp")
        .join("answered-watermarks")
        .join(format!("{agent_pubkey}-{}.json", &relay_scope[..16])))
}

fn persist_atomic(path: &Path, state: &WatermarkFile) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("watermark path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("answered-watermark"),
        std::process::id()
    ));
    let raw = serde_json::to_vec(state)?;
    let mut file = fs::File::create(&temp)?;
    file.write_all(&raw)?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipts_are_agent_relay_scoped_pruned_and_restart_durable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("watermark.json");
        let channel = Uuid::new_v4();
        let mut store =
            AnsweredWatermarkStore::open(Some(&path), &"a".repeat(64), "wss://relay.example")
                .expect("open store");
        store
            .merge_and_persist(
                [
                    (channel, "1".repeat(64), 100),
                    (channel, "2".repeat(64), 200),
                ],
                150,
            )
            .expect("persist");
        assert_eq!(store.answered_ids_since(0), HashSet::from(["2".repeat(64)]));
        assert_eq!(store.path(), path);

        let reopened =
            AnsweredWatermarkStore::open(Some(&path), &"a".repeat(64), "wss://relay.example/")
                .expect("reopen store");
        assert_eq!(
            reopened.answered_ids_since(150),
            HashSet::from(["2".repeat(64)])
        );
    }

    #[test]
    fn identity_or_relay_mismatch_fails_closed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("watermark.json");
        let mut store =
            AnsweredWatermarkStore::open(Some(&path), &"a".repeat(64), "wss://relay-a.example")
                .expect("open store");
        store
            .merge_and_persist([(Uuid::new_v4(), "1".repeat(64), 100)], 0)
            .expect("persist");

        assert!(AnsweredWatermarkStore::open(
            Some(&path),
            &"b".repeat(64),
            "wss://relay-a.example"
        )
        .is_err());
        assert!(AnsweredWatermarkStore::open(
            Some(&path),
            &"a".repeat(64),
            "wss://relay-b.example"
        )
        .is_err());
    }
}
