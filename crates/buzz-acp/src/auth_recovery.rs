//! Local, restart-durable authentication holds. Never stores provider credentials.
//! A restart re-arms the hold; only an explicit owner resume admits retained work.
use crate::{
    queue::{BatchEvent, CancelReason, FlushBatch},
    scope::SessionScope,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_BATCHES: usize = 512;

#[derive(Clone, Serialize, Deserialize)]
struct SavedBatch {
    channel: uuid::Uuid,
    root: Option<String>,
    events: Vec<(nostr::Event, String)>,
    cancelled: Vec<(nostr::Event, String)>,
    interrupt: Option<bool>,
}
impl SavedBatch {
    fn capture(batch: &FlushBatch) -> Self {
        Self {
            channel: batch.channel_id,
            root: match &batch.scope {
                SessionScope::Thread { root_event_id, .. } => Some(root_event_id.clone()),
                _ => None,
            },
            events: batch
                .events
                .iter()
                .map(|e| (e.event.clone(), e.prompt_tag.clone()))
                .collect(),
            cancelled: batch
                .cancelled_events
                .iter()
                .map(|e| (e.event.clone(), e.prompt_tag.clone()))
                .collect(),
            interrupt: batch.cancel_reason.map(|r| r == CancelReason::Interrupt),
        }
    }
    fn restore(&self) -> FlushBatch {
        let convert = |events: &Vec<(nostr::Event, String)>| {
            events
                .iter()
                .map(|(event, tag)| BatchEvent {
                    event: event.clone(),
                    prompt_tag: tag.clone(),
                    received_at: Instant::now(),
                })
                .collect()
        };
        FlushBatch {
            channel_id: self.channel,
            scope: self.root.as_ref().map_or(
                SessionScope::Conversation {
                    channel_id: self.channel,
                },
                |root| SessionScope::Thread {
                    channel_id: self.channel,
                    root_event_id: root.clone(),
                },
            ),
            events: convert(&self.events),
            cancelled_events: convert(&self.cancelled),
            cancel_reason: self.interrupt.map(|v| {
                if v {
                    CancelReason::Interrupt
                } else {
                    CancelReason::Steer
                }
            }),
            attempt: 1,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Journal {
    version: u32,
    identity: String,
    batches: Vec<SavedBatch>,
}

pub(crate) struct AuthRecovery {
    path: PathBuf,
    journal: Journal,
}
impl AuthRecovery {
    pub(crate) fn open(root: &Path, agent: &str, relay: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(
            root.is_absolute(),
            "authentication recovery root must be absolute"
        );
        for component in root.ancestors() {
            if let Ok(metadata) = fs::symlink_metadata(component) {
                anyhow::ensure!(
                    !metadata.file_type().is_symlink(),
                    "authentication recovery root contains a symlink"
                );
            }
        }
        fs::create_dir_all(root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
        }
        let identity = hex::encode(Sha256::digest(format!("{agent}\n{relay}").as_bytes()));
        let path = root.join(format!("{identity}.json"));
        let journal = match fs::symlink_metadata(&path) {
            Ok(meta) => {
                anyhow::ensure!(
                    meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= MAX_BYTES,
                    "invalid authentication recovery file"
                );
                let parsed: Journal = serde_json::from_slice(&fs::read(&path)?)?;
                anyhow::ensure!(
                    parsed.version == 1
                        && parsed.identity == identity
                        && parsed.batches.len() <= MAX_BATCHES,
                    "authentication recovery identity/version/size mismatch"
                );
                for batch in &parsed.batches {
                    for (event, _) in batch.events.iter().chain(&batch.cancelled) {
                        event.verify()?;
                    }
                }
                parsed
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Journal {
                version: 1,
                identity,
                batches: vec![],
            },
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path, journal })
    }
    pub(crate) fn batches(&self) -> Vec<FlushBatch> {
        self.journal
            .batches
            .iter()
            .map(SavedBatch::restore)
            .collect()
    }
    pub(crate) fn hold(&mut self, batch: &FlushBatch) -> anyhow::Result<()> {
        let mut next = self.journal.clone();
        let mut saved = SavedBatch::capture(batch);
        let mut ids: std::collections::HashSet<_> = saved
            .events
            .iter()
            .chain(&saved.cancelled)
            .map(|(event, _)| event.id)
            .collect();
        // A second failed attempt for this scope replaces its framing while
        // retaining every earlier unanswered original. Never duplicate the
        // same event merely because an interrupted request changed roles.
        for prior in next
            .batches
            .iter()
            .filter(|prior| prior.channel == saved.channel && prior.root == saved.root)
        {
            for event in &prior.events {
                if ids.insert(event.0.id) {
                    saved.events.push(event.clone());
                }
            }
            for event in &prior.cancelled {
                if ids.insert(event.0.id) {
                    saved.cancelled.push(event.clone());
                }
            }
        }
        next.batches
            .retain(|prior| prior.channel != saved.channel || prior.root != saved.root);
        next.batches.push(saved);
        anyhow::ensure!(
            next.batches.len() <= MAX_BATCHES,
            "authentication recovery journal full"
        );
        self.persist(next)
    }
    pub(crate) fn complete(
        &mut self,
        ids: &std::collections::HashSet<nostr::EventId>,
    ) -> anyhow::Result<()> {
        if !self
            .journal
            .batches
            .iter()
            .flat_map(|batch| batch.events.iter().chain(&batch.cancelled))
            .any(|(event, _)| ids.contains(&event.id))
        {
            return Ok(());
        }
        let mut next = self.journal.clone();
        for batch in &mut next.batches {
            batch.events.retain(|(event, _)| !ids.contains(&event.id));
            batch
                .cancelled
                .retain(|(event, _)| !ids.contains(&event.id));
        }
        next.batches.retain(|b| !b.events.is_empty());
        self.persist(next)
    }
    fn persist(&mut self, next: Journal) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec(&next)?;
        anyhow::ensure!(
            bytes.len() as u64 <= MAX_BYTES,
            "authentication recovery journal full"
        );
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let result = (|| -> anyhow::Result<()> {
            let mut file = options.open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)?;
            fs::File::open(self.path.parent().expect("owned root"))?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        self.journal = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::DedupMode, queue::EventQueue};

    fn batch() -> FlushBatch {
        let channel_id = uuid::Uuid::new_v4();
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(9), "retained request")
            .sign_with_keys(&nostr::Keys::generate())
            .unwrap();
        FlushBatch {
            channel_id,
            scope: SessionScope::Thread {
                channel_id,
                root_event_id: event.id.to_hex(),
            },
            events: vec![BatchEvent {
                event,
                prompt_tag: "mention".into(),
                received_at: Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
            attempt: 1,
        }
    }
    #[test]
    fn restart_requires_explicit_resume_and_completion_retires_original_id() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let original = batch();
        let mut queue = EventQueue::new(DedupMode::Queue)
            .with_auth_recovery(AuthRecovery::open(&root, "agent", "relay").unwrap());
        queue.hold_for_auth(original.clone()).unwrap();
        assert!(queue.auth_blocked());
        assert!(queue.flush_next().is_none());
        drop(queue);
        let mut queue = EventQueue::new(DedupMode::Queue)
            .with_auth_recovery(AuthRecovery::open(&root, "agent", "relay").unwrap());
        assert!(queue.flush_next().is_none());
        assert_eq!(queue.resume_after_auth().unwrap(), 1);
        let retried = queue.flush_next().unwrap();
        assert_eq!(retried.scope, original.scope);
        assert_eq!(retried.events[0].event.id, original.events[0].event.id);
        // Dispatch is not completion: a crash still preserves the request.
        assert_eq!(
            AuthRecovery::open(&root, "agent", "relay")
                .unwrap()
                .batches()
                .len(),
            1
        );
        queue.complete_auth_recovery(&retried).unwrap();
        assert!(AuthRecovery::open(&root, "agent", "relay")
            .unwrap()
            .batches()
            .is_empty());
        assert!(AuthRecovery::open(&root, "other-agent", "relay")
            .unwrap()
            .batches()
            .is_empty());
    }
    #[test]
    fn cancelled_carryover_is_replay_fenced_before_and_after_restart() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let mut original = batch();
        original.cancelled_events = batch().events;
        original.cancel_reason = Some(CancelReason::Interrupt);
        let replay = |event: &BatchEvent| crate::queue::QueuedEvent {
            channel_id: original.channel_id,
            scope: original.scope.clone(),
            event: event.event.clone(),
            prompt_tag: event.prompt_tag.clone(),
            received_at: Instant::now(),
        };
        let mut queue = EventQueue::new(DedupMode::Queue)
            .with_auth_recovery(AuthRecovery::open(&root, "agent", "relay").unwrap());
        queue.hold_for_auth(original.clone()).unwrap();
        for event in original.events.iter().chain(&original.cancelled_events) {
            assert!(
                !queue.push(replay(event)),
                "held originals must not duplicate through relay replay"
            );
        }
        drop(queue);
        let mut queue = EventQueue::new(DedupMode::Queue)
            .with_auth_recovery(AuthRecovery::open(&root, "agent", "relay").unwrap());
        for event in original.events.iter().chain(&original.cancelled_events) {
            assert!(
                !queue.push(replay(event)),
                "restart must restore every replay fence"
            );
        }
        queue.resume_after_auth().unwrap();
        for event in original.events.iter().chain(&original.cancelled_events) {
            assert!(
                !queue.push(replay(event)),
                "resume must retain every replay fence until completion"
            );
        }
        let resumed = queue.flush_next().unwrap();
        assert_eq!(resumed.events.len(), 1);
        assert_eq!(resumed.cancelled_events.len(), 1);
        assert_eq!(
            resumed.cancelled_events[0].event.id,
            original.cancelled_events[0].event.id
        );
        assert_eq!(resumed.cancel_reason, original.cancel_reason);
        queue.complete_auth_recovery(&resumed).unwrap();
        assert!(AuthRecovery::open(&root, "agent", "relay")
            .unwrap()
            .batches()
            .is_empty());
    }

    #[test]
    fn cancelled_receipt_is_retired_without_discarding_unanswered_primary() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let mut original = batch();
        original.cancelled_events = batch().events;
        let mut recovery = AuthRecovery::open(&root, "agent", "relay").unwrap();
        recovery.hold(&original).unwrap();
        recovery
            .complete(&std::collections::HashSet::from([original
                .cancelled_events[0]
                .event
                .id]))
            .unwrap();
        let stored = AuthRecovery::open(&root, "agent", "relay")
            .unwrap()
            .batches();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].events[0].event.id, original.events[0].event.id);
        assert!(stored[0].cancelled_events.is_empty());
    }

    #[test]
    fn malformed_journal_fails_closed_and_failed_write_keeps_memory_hold() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let recovery = AuthRecovery::open(&root, "agent", "relay").unwrap();
        fs::write(&recovery.path, b"invalid").unwrap();
        assert!(AuthRecovery::open(&root, "agent", "relay").is_err());
        fs::remove_file(&recovery.path).unwrap();
        let mut queue = EventQueue::new(DedupMode::Queue).with_auth_recovery(recovery);
        fs::remove_dir(&root).unwrap();
        assert!(queue.hold_for_auth(batch()).is_err());
        assert!(queue.auth_blocked());
        assert!(queue.flush_next().is_none());
        assert!(queue.resume_after_auth().is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_journal_rejected_without_reading_target() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let recovery = AuthRecovery::open(&root, "agent", "relay").unwrap();
        std::os::unix::fs::symlink(root.join("missing"), &recovery.path).unwrap();
        assert!(AuthRecovery::open(&root, "agent", "relay").is_err());
    }
}
