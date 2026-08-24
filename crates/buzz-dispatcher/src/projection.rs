//! Persistent local projection state for the dispatcher.
//!
//! The dispatcher keeps a resident local store instead of refetching the full
//! stream per command (plan: M15 ban). One append-only JSONL file holds every
//! raw relay event the dispatcher has ingested (tracker stream events, pin
//! reactions, dispatch actions); a single composite `(created_at, event_id)`
//! cursor marks the high-water point across all watched families.
//!
//! Refresh contract (inclusive refresh): the poll loop queries each family with
//! `since = cursor.created_at` (inclusive), so a boundary event can arrive
//! twice; [`ProjectionStore::ingest`] dedupes by event id and keeps the store
//! sorted by `(created_at, event_id)`. A restart re-opens the store from disk
//! and resumes from the persisted cursor with no gaps or duplicate effects.

use std::collections::BTreeSet;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use nostr::{Event, JsonUtil, Kind};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::DispatchError;

const CURSOR_FILE: &str = "cursor.json";
const EVENTS_FILE: &str = "events.jsonl";
const REQUESTS_FILE: &str = "requests.json";

/// Composite `(created_at, event_id)` high-water cursor.
///
/// Ordering is lexicographic on `(created_at, event_id)`, matching the
/// deterministic order used by the tracker fold, so same-second events are
/// totally ordered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Cursor {
    /// Event creation time of the newest ingested event (0 = nothing yet).
    created_at: u64,
    /// Event id of the newest ingested event (empty = nothing yet).
    event_id: String,
}

impl Cursor {
    fn initial() -> Self {
        Self {
            created_at: 0,
            event_id: String::new(),
        }
    }

    /// True when `(created_at, event_id)` is strictly newer than the cursor.
    fn precedes(&self, created_at: u64, event_id: &str) -> bool {
        (self.created_at, self.event_id.as_str()) < (created_at, event_id)
    }
}

/// Set of fired dispatch request ids, persisted across restarts so
/// at-most-once semantics survive crashes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct FiredRequests {
    #[serde(default)]
    fired: BTreeSet<Uuid>,
}

/// Persistent projection store: raw event journal + composite cursor +
/// fired-request ledger, all under one config-named directory outside the repo.
#[derive(Debug)]
pub struct ProjectionStore {
    state_dir: PathBuf,
    cursor: Cursor,
    events: Vec<Event>,
    seen_ids: HashSet<String>,
    fired: FiredRequests,
    dirty: bool,
}

impl ProjectionStore {
    /// Open (or create) the store at `state_dir`. Missing files start empty;
    /// a corrupt journal or cursor is a hard error rather than a silent reset.
    pub fn open(state_dir: &Path) -> Result<Self, DispatchError> {
        fs::create_dir_all(state_dir)?;

        let cursor_path = state_dir.join(CURSOR_FILE);
        let cursor = if cursor_path.exists() {
            serde_json::from_str(&fs::read_to_string(&cursor_path)?)
                .map_err(|error| DispatchError::Other(format!("corrupt cursor file: {error}")))?
        } else {
            Cursor::initial()
        };

        let mut events = Vec::new();
        let mut seen_ids = HashSet::new();
        let events_path = state_dir.join(EVENTS_FILE);
        if events_path.exists() {
            for (line_no, line) in fs::read_to_string(&events_path)?.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let event: Event = serde_json::from_str(line).map_err(|error| {
                    DispatchError::Other(format!("events.jsonl line {line_no}: {error}"))
                })?;
                seen_ids.insert(event.id.to_hex());
                events.push(event);
            }
        }

        let requests_path = state_dir.join(REQUESTS_FILE);
        let fired = if requests_path.exists() {
            serde_json::from_str(&fs::read_to_string(&requests_path)?)
                .map_err(|error| DispatchError::Other(format!("corrupt requests file: {error}")))?
        } else {
            FiredRequests::default()
        };

        Ok(Self {
            state_dir: state_dir.to_path_buf(),
            cursor,
            events,
            seen_ids,
            fired,
            dirty: false,
        })
    }

    /// Current composite cursor.
    pub fn cursor(&self) -> (u64, &str) {
        (self.cursor.created_at, &self.cursor.event_id)
    }

    /// `since` value for an inclusive refresh query (seconds).
    pub fn since(&self) -> u64 {
        self.cursor.created_at
    }

    /// Cursor serialized for the heartbeat `cursor` field, bounded to
    /// [`buzz_sdk::tracker::MAX_CURSOR_BYTES`].
    pub fn cursor_payload(&self) -> Result<String, DispatchError> {
        let payload = serde_json::to_string(&self.cursor)?;
        if payload.len() > buzz_sdk::tracker::MAX_CURSOR_BYTES {
            return Err(DispatchError::Other(format!(
                "cursor payload exceeds {} bytes",
                buzz_sdk::tracker::MAX_CURSOR_BYTES
            )));
        }
        Ok(payload)
    }

    /// All persisted raw events in `(created_at, event_id)` order.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Ingest a batch from one inclusive refresh. Events already in the store
    /// (event-id dedupe) or not strictly newer than the cursor are dropped.
    /// Returns the number of events newly accepted. Nothing touches disk until
    /// [`ProjectionStore::commit`].
    pub fn ingest(&mut self, batch: Vec<Event>) -> usize {
        let mut accepted = Vec::new();
        for event in batch {
            let id = event.id.to_hex();
            if self.seen_ids.contains(&id) {
                continue;
            }
            accepted.push(event);
        }
        if accepted.is_empty() {
            return 0;
        }
        let newly = accepted.len();
        for event in &accepted {
            self.seen_ids.insert(event.id.to_hex());
            let created_at = event.created_at.as_secs();
            let id = event.id.to_hex();
            if self.cursor.precedes(created_at, &id) {
                self.cursor = Cursor {
                    created_at,
                    event_id: id,
                };
            }
        }
        self.events.append(&mut accepted);
        // Maintain the global (created_at, event_id) order invariant.
        self.events.sort_by(|a, b| {
            (a.created_at.as_secs(), a.id.to_hex()).cmp(&(b.created_at.as_secs(), b.id.to_hex()))
        });
        self.dirty = true;
        newly
    }

    /// True when some persisted kind-40020 root already captured
    /// `source_event_id`. Belt-and-suspenders idempotency for pin capture:
    /// even if UUIDv5 request-id namespaces drift, a second root for the same
    /// pinned message is blocked.
    pub fn has_root_with_source(&self, source_event_id: &str) -> bool {
        let kind = Kind::Custom(buzz_sdk::kind::KIND_TRACKER_ISSUE as u16);
        self.events.iter().any(|event| {
            if event.kind != kind {
                return false;
            }
            serde_json::from_str::<buzz_sdk::tracker::IssueRoot>(&event.content)
                .map(|root| root.source_event_id.as_deref() == Some(source_event_id))
                .unwrap_or(false)
        })
    }

    /// True when a dispatch request id was already fired (by this dispatcher
    /// or observed via a parsed 40023 pre-check).
    pub fn request_fired(&self, request_id: &Uuid) -> bool {
        self.fired.fired.contains(request_id)
    }

    /// Issue UUIDs whose root or any revision carries import provenance.
    /// M19: imported issues never trigger dispatcher rules.
    pub fn imported_issue_ids(&self) -> HashSet<Uuid> {
        let root_kind = Kind::Custom(buzz_sdk::kind::KIND_TRACKER_ISSUE as u16);
        let rev_kind = Kind::Custom(buzz_sdk::kind::KIND_TRACKER_REVISION as u16);
        let mut suppressed = HashSet::new();
        for event in &self.events {
            if event.kind == root_kind {
                if let Ok(root) =
                    serde_json::from_str::<buzz_sdk::tracker::IssueRoot>(&event.content)
                {
                    if root.import.is_some() {
                        suppressed.insert(root.issue_id);
                    }
                }
            } else if event.kind == rev_kind {
                if let Ok(revision) =
                    serde_json::from_str::<buzz_sdk::tracker::IssueRevision>(&event.content)
                {
                    if revision.import.is_some() {
                        suppressed.insert(revision.issue_id);
                    }
                }
            }
        }
        suppressed
    }
    /// Record a fired dispatch request id.
    pub fn mark_request_fired(&mut self, request_id: Uuid) {
        if self.fired.fired.insert(request_id) {
            self.dirty = true;
        }
    }

    /// Observe externally fired request ids (e.g. 40023s authored by another
    /// service) so the at-most-once pre-check covers them too.
    pub fn observe_fired_requests(&mut self, request_ids: impl IntoIterator<Item = Uuid>) {
        for id in request_ids {
            if self.fired.fired.insert(id) {
                self.dirty = true;
            }
        }
    }

    /// Whether anything changed since open/last commit.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Flush journal, cursor, and fired-request ledger to disk. Each file is
    /// written to a temp sibling and renamed, so a crash mid-write never
    /// leaves a torn state file.
    pub fn commit(&mut self) -> Result<(), DispatchError> {
        if !self.dirty {
            return Ok(());
        }

        let mut journal = String::new();
        for event in &self.events {
            journal.push_str(&event.as_json());
            journal.push('\n');
        }
        write_atomic(&self.state_dir.join(EVENTS_FILE), journal.as_bytes())?;
        write_atomic(
            &self.state_dir.join(CURSOR_FILE),
            serde_json::to_string_pretty(&self.cursor)?.as_bytes(),
        )?;
        write_atomic(
            &self.state_dir.join(REQUESTS_FILE),
            serde_json::to_string(&self.fired)?.as_bytes(),
        )?;
        self.dirty = false;
        Ok(())
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), DispatchError> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use buzz_sdk::tracker::{build_tracker_issue, IssueRoot, IssueStatus};
    use nostr::{EventBuilder, Keys, Timestamp};

    fn signed_event(created_at: u64, content: &str) -> Event {
        let keys = Keys::generate();
        EventBuilder::new(Kind::TextNote, content)
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(&keys)
            .expect("signed")
    }

    pub(crate) fn sample_root_event(source_event_id: Option<&str>) -> Event {
        sample_root_event_for(Uuid::new_v4(), source_event_id, false)
    }

    pub(crate) fn imported_root_event(issue_id: Uuid) -> Event {
        sample_root_event_for(issue_id, Some(&"e".repeat(64)), true)
    }

    fn sample_root_event_for(
        issue_id: Uuid,
        source_event_id: Option<&str>,
        imported: bool,
    ) -> Event {
        let keys = Keys::generate();
        let import = imported.then(|| buzz_sdk::tracker::ImportProvenance {
            system: "legacy".to_string(),
            external_id: "EXT-1".to_string(),
            source_url: None,
            imported_at: 1,
            source_created_at: None,
            source_updated_at: None,
            source_completed_at: None,
            source_cancelled_at: None,
            archived: None,
            estimate_points: None,
            export_sha256: None,
            importer_version: None,
            original_author: None,
            legacy_labels: Vec::new(),
        });
        let root = IssueRoot {
            v: buzz_sdk::tracker::TRACKER_VERSION,
            op: "create".to_string(),
            issue_id,
            identifier: "WS-1".to_string(),
            title: "pinned".to_string(),
            body: "captured from pin".to_string(),
            status: IssueStatus::Triage,
            priority: None,
            assignee: None,
            project_id: None,
            due: None,
            estimate_minutes: None,
            home_channel_id: None,
            source_event_id: source_event_id.map(str::to_string),
            workflow: "default".to_string(),
            labels: Vec::new(),
            external_refs: Vec::new(),
            request_id: Some(buzz_sdk::tracker::pin_capture_request_id(
                source_event_id.unwrap_or("none"),
            )),
            import,
        };
        let builder = build_tracker_issue(Uuid::new_v4(), &root).expect("built");
        builder.sign_with_keys(&keys).expect("signed")
    }

    #[test]
    fn ingest_dedupes_and_orders_same_second_events() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = ProjectionStore::open(dir.path()).expect("open");

        // Same created_at — ordering must fall back to event id.
        let a = signed_event(100, "a");
        let b = signed_event(100, "b");
        let c = signed_event(101, "c");

        store.ingest(vec![c.clone(), b.clone(), a.clone()]);
        let ids: Vec<String> = store.events().iter().map(|e| e.id.to_hex()).collect();
        let mut expected: Vec<(u64, String)> = vec![
            (a.created_at.as_secs(), a.id.to_hex()),
            (b.created_at.as_secs(), b.id.to_hex()),
            (c.created_at.as_secs(), c.id.to_hex()),
        ];
        expected.sort();
        let expected_ids: Vec<String> = expected.into_iter().map(|(_, id)| id).collect();
        assert_eq!(
            ids, expected_ids,
            "store must be (created_at, event_id) ordered"
        );

        // Duplicate batch: nothing accepted, cursor unchanged.
        let before = (store.cursor().0, store.cursor().1.to_string());
        let accepted = store.ingest(vec![a, b, c]);
        assert_eq!(accepted, 0);
        assert_eq!((store.cursor().0, store.cursor().1.to_string()), before);
    }

    #[test]
    fn resume_after_restart_produces_no_gaps_or_dupes() {
        let dir = tempfile::tempdir().expect("tempdir");

        let first = signed_event(50, "first");
        let second = signed_event(60, "second");

        {
            let mut store = ProjectionStore::open(dir.path()).expect("open");
            assert_eq!(store.ingest(vec![first.clone(), second.clone()]), 2);
            store.mark_request_fired(Uuid::new_v4());
            store.commit().expect("commit");
        }

        // Restart: an inclusive refresh re-delivers `second` (boundary) and a
        // genuinely newer event.
        let third = signed_event(70, "third");
        let mut store = ProjectionStore::open(dir.path()).expect("reopen");
        assert_eq!(store.events().len(), 2, "journal survives restart");
        assert!(store.cursor().0 == 60, "cursor survives restart");

        let accepted = store.ingest(vec![second.clone(), third.clone()]);
        assert_eq!(accepted, 1, "boundary duplicate deduped, new event taken");
        assert_eq!(store.events().len(), 3);
        assert_eq!(store.cursor().0, 70);
        store.commit().expect("commit");

        // Third generation: no dupes after another restart.
        let store = ProjectionStore::open(dir.path()).expect("reopen 2");
        assert_eq!(store.events().len(), 3);
    }

    #[test]
    fn root_source_precheck_blocks_duplicate_capture() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = ProjectionStore::open(dir.path()).expect("open");

        let source = "a".repeat(64);
        let other = "b".repeat(64);
        assert!(!store.has_root_with_source(&source));
        store.ingest(vec![sample_root_event(Some(&source))]);
        assert!(store.has_root_with_source(&source));
        assert!(!store.has_root_with_source(&other));
    }

    #[test]
    fn cursor_payload_is_bounded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = ProjectionStore::open(dir.path()).expect("open");
        store.ingest(vec![signed_event(42, "x")]);
        let payload = store.cursor_payload().expect("payload");
        assert!(payload.len() <= buzz_sdk::tracker::MAX_CURSOR_BYTES);
        assert!(payload.contains("42"));
    }

    #[test]
    fn fired_requests_survive_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let request = Uuid::new_v4();
        {
            let mut store = ProjectionStore::open(dir.path()).expect("open");
            assert!(!store.request_fired(&request));
            store.mark_request_fired(request);
            store.commit().expect("commit");
        }
        let store = ProjectionStore::open(dir.path()).expect("reopen");
        assert!(store.request_fired(&request));
    }

    #[test]
    fn corrupt_journal_is_a_hard_error_not_a_reset() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join(EVENTS_FILE), "not-json\n").expect("write");
        let error = ProjectionStore::open(dir.path()).expect_err("must fail");
        assert!(matches!(error, DispatchError::Other(_)));
    }
}
