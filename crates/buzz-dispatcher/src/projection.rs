//! Persistent local projection state for the dispatcher.
//!
//! The dispatcher keeps a resident local store instead of refetching the full
//! stream per command (plan: M15 ban). One JSONL journal file holds every raw
//! relay event the dispatcher has ingested (tracker stream events, pin
//! reactions, dispatch actions); a single composite `(created_at, event_id)`
//! cursor marks the high-water point across all watched families.
//!
//! Refresh contract (inclusive refresh): the poll loop queries each family with
//! `since = cursor.created_at` (inclusive), so a boundary event can arrive
//! twice; [`ProjectionStore::ingest`] dedupes by event id and keeps the store
//! sorted by `(created_at, event_id)`. A restart re-opens the store from disk
//! and resumes from the persisted cursor with no gaps or duplicate effects.
//!
//! Durability contract: state files are written to a temp sibling, `fsync`ed,
//! and renamed, so a crash mid-write never leaves a torn file. A corrupt state
//! file is a HARD ERROR at open, never a silent reset. Recovery from a corrupt
//! (or lost) state dir: stop the service, move the whole `state_dir` aside,
//! restart — the dispatcher re-ingests from the relay's retained history and
//! its request-ID pre-checks keep replays idempotent (see docs/dispatcher.md).

use std::collections::BTreeSet;
use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use nostr::{Event, JsonUtil, Kind};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::DispatchError;

const CURSOR_FILE: &str = "cursor.json";
const EVENTS_FILE: &str = "events.jsonl";
const REQUESTS_FILE: &str = "requests.json";
const OUTBOX_FILE: &str = "outbox.json";
/// Single-instance lock for the resident loop (M7).
pub const DISPATCHER_LOCK_FILE: &str = "dispatcher.lock";
/// Single-instance lock for the watchdog process (M7).
pub const WATCHDOG_LOCK_FILE: &str = "watchdog.lock";

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

/// One pending publication: the signed events for a request id, persisted
/// BEFORE publish (M2 persist-before-publish). A crash between relay accept
/// and ledger commit replays from here on restart instead of re-planning from
/// scratch, so each request stays at-most-once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutboxEntry {
    /// Request id the publication belongs to.
    pub request_id: Uuid,
    /// Signed events (serialized JSON) in publication order.
    pub events: Vec<serde_json::Value>,
}

/// Pending publications awaiting relay submission.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Outbox {
    #[serde(default)]
    pending: Vec<OutboxEntry>,
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
    outbox: Outbox,
    /// Journal/cursor changed (new events ingested) — the only condition that
    /// rewrites events.jsonl (M8 cheap win).
    journal_dirty: bool,
    /// Fired-request ledger changed.
    ledger_dirty: bool,
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

        let outbox_path = state_dir.join(OUTBOX_FILE);
        let outbox = if outbox_path.exists() {
            serde_json::from_str(&fs::read_to_string(&outbox_path)?)
                .map_err(|error| DispatchError::Other(format!("corrupt outbox file: {error}")))?
        } else {
            Outbox::default()
        };

        Ok(Self {
            state_dir: state_dir.to_path_buf(),
            cursor,
            events,
            seen_ids,
            fired,
            outbox,
            journal_dirty: false,
            ledger_dirty: false,
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
    /// (event-id dedupe) are dropped. Returns the number of events newly
    /// accepted. Nothing touches disk until [`ProjectionStore::commit`].
    ///
    /// M1: cursor advancement is clamped to `min(created_at, now)`. The relay
    /// accepts ~15 minutes of future skew, and an unclamped publisher-supplied
    /// timestamp would advance the cursor into the future, blinding the
    /// refresh window until local time caught up (repeatably). The clamped
    /// event is still journaled and id-deduped — only the high-water mark is
    /// bounded by local time.
    pub fn ingest(&mut self, batch: Vec<Event>, now: u64) -> usize {
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
            // M1 clamp: never advance past local time.
            let created_at = event.created_at.as_secs().min(now);
            let id = event.id.to_hex();
            if self.cursor.precedes(created_at, &id) {
                self.cursor = Cursor {
                    created_at,
                    event_id: id,
                };
            }
        }
        // M8 cheap win: incremental sorted merge instead of a full re-sort of
        // the whole journal per ingest.
        accepted.sort_unstable_by_key(event_key);
        let existing = std::mem::take(&mut self.events);
        self.events = merge_sorted_events(existing, accepted);
        self.journal_dirty = true;
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
            self.ledger_dirty = true;
        }
    }

    /// Observe externally fired request ids (e.g. 40023s authored by another
    /// service) so the at-most-once pre-check covers them too.
    pub fn observe_fired_requests(&mut self, request_ids: impl IntoIterator<Item = Uuid>) {
        for id in request_ids {
            if self.fired.fired.insert(id) {
                self.ledger_dirty = true;
            }
        }
    }

    /// Persist the fired ledger to disk immediately (the publish pipeline
    /// needs it durable BEFORE the matching outbox entry is cleared).
    pub fn persist_ledger(&mut self) -> Result<(), DispatchError> {
        write_atomic(
            &self.state_dir.join(REQUESTS_FILE),
            serde_json::to_string(&self.fired)?.as_bytes(),
        )?;
        self.ledger_dirty = false;
        Ok(())
    }

    /// Pending outbox entries awaiting replay/publication.
    pub fn pending_outbox(&self) -> &[OutboxEntry] {
        &self.outbox.pending
    }

    /// Persist one pending publication BEFORE it is published (M2). Any
    /// existing entry for the same request id is replaced (a failed publish
    /// re-queues, never duplicates).
    pub fn queue_outbox(
        &mut self,
        request_id: Uuid,
        events: Vec<serde_json::Value>,
    ) -> Result<(), DispatchError> {
        self.outbox
            .pending
            .retain(|entry| entry.request_id != request_id);
        self.outbox.pending.push(OutboxEntry { request_id, events });
        self.write_outbox()
    }

    /// Remove a completed publication from the outbox and persist. Call this
    /// only AFTER [`Self::mark_request_fired`] + [`Self::persist_ledger`]:
    /// a crash in between replays the entry (duplicate-tolerant) rather than
    /// losing the fired mark.
    pub fn complete_outbox(&mut self, request_id: &Uuid) -> Result<(), DispatchError> {
        let before = self.outbox.pending.len();
        self.outbox
            .pending
            .retain(|entry| &entry.request_id != request_id);
        if self.outbox.pending.len() != before {
            self.write_outbox()?;
        }
        Ok(())
    }

    fn write_outbox(&self) -> Result<(), DispatchError> {
        write_atomic(
            &self.state_dir.join(OUTBOX_FILE),
            serde_json::to_string(&self.outbox)?.as_bytes(),
        )
    }

    /// Whether anything changed since open/last commit.
    pub fn is_dirty(&self) -> bool {
        self.journal_dirty || self.ledger_dirty
    }

    /// Flush changed state to disk. The journal + cursor are rewritten ONLY
    /// when new events were ingested, and the fired-request ledger ONLY when
    /// it changed (M8 cheap win: a ledger-only tick no longer rewrites the
    /// whole journal). Each file is written to a temp sibling, fsynced, and
    /// renamed, so a crash mid-write never leaves a torn state file.
    pub fn commit(&mut self) -> Result<(), DispatchError> {
        if self.journal_dirty {
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
            self.journal_dirty = false;
        }
        if self.ledger_dirty {
            write_atomic(
                &self.state_dir.join(REQUESTS_FILE),
                serde_json::to_string(&self.fired)?.as_bytes(),
            )?;
            self.ledger_dirty = false;
        }
        Ok(())
    }
}

/// Single-instance advisory lock over one file in the state dir (M7).
///
/// Two dispatcher processes rename-clobbering `events.jsonl`/`requests.json`
/// last-writer-wins would silently lose fired-ledger entries and produce
/// duplicate emissions. The resident loop and the watchdog each hold their
/// own lock file, so a second instance (manual run + launchd, or a respawn
/// during a wedge) fails fast instead of corrupting state. Released on drop.
///
/// Uses flock via the `fs2` crate (exclusive, non-blocking). The only
/// Cargo.lock delta is the fs2 entry itself — the lock was hand-pinned so no
/// unrelated resolution moved.
pub struct InstanceLock {
    _file: fs::File,
}

impl InstanceLock {
    /// Acquire an exclusive non-blocking lock on `state_dir/lock_name`.
    pub fn acquire(state_dir: &Path, lock_name: &str) -> Result<Self, DispatchError> {
        fs::create_dir_all(state_dir)?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(state_dir.join(lock_name))?;
        file.try_lock_exclusive().map_err(|error| {
            DispatchError::Other(format!(
                "another instance holds {} in {}: {error}",
                lock_name,
                state_dir.display()
            ))
        })?;
        Ok(Self { _file: file })
    }
}

/// `(created_at, id)` sort key shared by ingest ordering and the merge.
fn event_key(event: &Event) -> (u64, String) {
    (event.created_at.as_secs(), event.id.to_hex())
}

/// Merge two `(created_at, event_id)`-sorted event vectors in one pass.
fn merge_sorted_events(a: Vec<Event>, b: Vec<Event>) -> Vec<Event> {
    if a.is_empty() {
        return b;
    }
    if b.is_empty() {
        return a;
    }
    let mut out = Vec::with_capacity(a.len() + b.len());
    let mut a_iter = a.into_iter().peekable();
    let mut b_iter = b.into_iter().peekable();
    loop {
        let take_a = match (a_iter.peek(), b_iter.peek()) {
            (Some(left), Some(right)) => event_key(left) <= event_key(right),
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        if let Some(event) = if take_a { a_iter.next() } else { b_iter.next() } {
            out.push(event);
        }
    }
    out
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), DispatchError> {
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    // fsync the data before the rename so the renamed file is durable, not a
    // zero-length/torn placeholder the corrupt-state hard-error would trip on
    // after a power loss.
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use buzz_sdk::tracker::{build_tracker_issue, IssueRoot, IssueStatus};
    use nostr::{EventBuilder, Keys, Timestamp};

    /// Test clock: comfortably above every fixture created_at.
    pub(crate) const TEST_NOW: u64 = 2_000_000_000;

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

        store.ingest(vec![c.clone(), b.clone(), a.clone()], TEST_NOW);
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
        let accepted = store.ingest(vec![a, b, c], TEST_NOW);
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
            assert_eq!(
                store.ingest(vec![first.clone(), second.clone()], TEST_NOW),
                2
            );
            store.mark_request_fired(Uuid::new_v4());
            store.commit().expect("commit");
        }

        // Restart: an inclusive refresh re-delivers `second` (boundary) and a
        // genuinely newer event.
        let third = signed_event(70, "third");
        let mut store = ProjectionStore::open(dir.path()).expect("reopen");
        assert_eq!(store.events().len(), 2, "journal survives restart");
        assert!(store.cursor().0 == 60, "cursor survives restart");

        let accepted = store.ingest(vec![second.clone(), third.clone()], TEST_NOW);
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
        store.ingest(vec![sample_root_event(Some(&source))], TEST_NOW);
        assert!(store.has_root_with_source(&source));
        assert!(!store.has_root_with_source(&other));
    }

    #[test]
    fn cursor_payload_is_bounded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = ProjectionStore::open(dir.path()).expect("open");
        store.ingest(vec![signed_event(42, "x")], TEST_NOW);
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

    /// M1 regression guard: publisher-controlled `created_at` may run up to
    /// ~15 minutes ahead (relay skew tolerance). The cursor must clamp to
    /// local `now` so one future-stamped event cannot blind the refresh
    /// window; the event itself is still journaled and deduped.
    #[test]
    fn cursor_advancement_is_clamped_to_now() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = ProjectionStore::open(dir.path()).expect("open");

        let future = signed_event(9_999_999, "future-stamped");
        let local_now = 1_000;
        assert_eq!(store.ingest(vec![future.clone()], local_now), 1);
        assert_eq!(
            store.cursor().0,
            local_now,
            "cursor clamps future created_at to local now"
        );
        assert_eq!(store.events().len(), 1, "event still journaled");

        // Re-delivery dedupes by id even though the cursor is clamped.
        assert_eq!(store.ingest(vec![future], local_now + 10), 0);

        // A genuinely newer (but still clamped) event advances within bounds.
        let normal = signed_event(1_500, "normal");
        assert_eq!(store.ingest(vec![normal], 2_000), 1);
        assert_eq!(store.cursor().0, 1_500);
    }

    /// M7 regression guard: a second instance against the same state dir
    /// fails fast instead of clobbering state files last-writer-wins.
    #[test]
    fn instance_lock_blocks_second_acquisition() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = InstanceLock::acquire(dir.path(), DISPATCHER_LOCK_FILE).expect("first lock");
        let second = InstanceLock::acquire(dir.path(), DISPATCHER_LOCK_FILE);
        assert!(
            second.is_err(),
            "second instance must not acquire the same lock"
        );
        drop(first);
        InstanceLock::acquire(dir.path(), DISPATCHER_LOCK_FILE).expect("lock released on drop");
        // The watchdog lock is independent of the dispatcher lock.
        InstanceLock::acquire(dir.path(), WATCHDOG_LOCK_FILE).expect("watchdog lock");
    }

    /// M8 cheap win: a ledger-only change must not rewrite events.jsonl.
    #[test]
    fn commit_skips_journal_rewrite_when_only_ledger_changed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = ProjectionStore::open(dir.path()).expect("open");
        store.ingest(vec![signed_event(100, "x")], TEST_NOW);
        store.commit().expect("commit");

        // Prove the skip deterministically: remove the journal, then a
        // ledger-only commit must not recreate it.
        let journal_path = dir.path().join(EVENTS_FILE);
        fs::remove_file(&journal_path).expect("remove journal");
        store.mark_request_fired(Uuid::new_v4());
        assert!(store.is_dirty());
        store.commit().expect("ledger commit");
        assert!(
            !journal_path.exists(),
            "ledger-only commit must not rewrite the journal"
        );
        assert!(dir.path().join(REQUESTS_FILE).exists());
        assert!(!store.is_dirty());
    }

    /// M2 outbox lifecycle: entries persist across restart until completed,
    /// re-queueing one request replaces (never duplicates) its entry, and
    /// completion removes it durably.
    #[test]
    fn outbox_round_trip_survives_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let request = Uuid::new_v4();
        let payload = serde_json::json!({ "id": "signed-event-bytes" });

        {
            let mut store = ProjectionStore::open(dir.path()).expect("open");
            assert!(store.pending_outbox().is_empty());
            store
                .queue_outbox(request, vec![payload.clone()])
                .expect("queue");
            // Re-queue replaces, not duplicates.
            store
                .queue_outbox(request, vec![payload.clone()])
                .expect("requeue");
            assert_eq!(store.pending_outbox().len(), 1);
        }

        // Restart: the pending publication must survive for replay.
        let mut store = ProjectionStore::open(dir.path()).expect("reopen");
        assert_eq!(store.pending_outbox().len(), 1);
        assert_eq!(store.pending_outbox()[0].request_id, request);
        assert_eq!(store.pending_outbox()[0].events, vec![payload]);

        store.mark_request_fired(request);
        store.persist_ledger().expect("ledger");
        store.complete_outbox(&request).expect("complete");
        assert!(store.pending_outbox().is_empty());

        let store = ProjectionStore::open(dir.path()).expect("reopen 2");
        assert!(store.pending_outbox().is_empty());
        assert!(store.request_fired(&request));
    }

    #[test]
    fn corrupt_outbox_is_a_hard_error_not_a_reset() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join(OUTBOX_FILE), "not-json\n").expect("write");
        let error = ProjectionStore::open(dir.path()).expect_err("must fail");
        assert!(matches!(error, DispatchError::Other(_)));
    }
}
