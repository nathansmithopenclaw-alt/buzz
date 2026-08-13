//! Automatic harness resurrection for stopped managed agents.
//!
//! Desktop must not hot-loop a dying harness, must not revive archived
//! (`is_active = false`) agents, and must not spawn for an inbound event
//! the agent would refuse under `respond_to`. Those decisions live here so
//! spawn, apply-update, and the inbound mention hook share one predicate.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::{ManagedAgentRecord, RespondTo};

/// Exit inside this window after an automatic spawn counts as an instant death.
pub const INSTANT_DEATH_WINDOW: Duration = Duration::from_secs(5);
/// Automatic respawn attempts before the owner must intervene.
pub const CRASH_LOOP_CAP: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundRespawnDecision {
    Spawn,
    SkipAlreadyRunning,
    SkipInactive,
    SkipNotLocal,
    SkipSelfAuthored,
    SkipAuthorGated,
    SkipCrashBackoff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyRestartDecision {
    Start,
    SkipAlreadyRunning,
    SkipInactive,
    SkipNotLocal,
    SkipCrashBackoff,
}

#[derive(Debug, Default)]
struct CrashWatch {
    auto_spawned_at: Option<Instant>,
    consecutive_instant_deaths: u32,
}

fn crash_watches() -> &'static Mutex<HashMap<String, CrashWatch>> {
    static CRASH_WATCHES: OnceLock<Mutex<HashMap<String, CrashWatch>>> = OnceLock::new();
    CRASH_WATCHES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn watch_key(pubkey: &str) -> String {
    pubkey.to_ascii_lowercase()
}

#[cfg(test)]
pub fn reset_crash_watches_for_tests() {
    if let Ok(mut watches) = crash_watches().lock() {
        watches.clear();
    }
}

/// Record that an automatic (inbound / apply-update) spawn just happened.
pub fn note_automatic_spawn(pubkey: &str) {
    if let Ok(mut watches) = crash_watches().lock() {
        watches
            .entry(watch_key(pubkey))
            .or_default()
            .auto_spawned_at = Some(Instant::now());
    }
}

/// Owner-initiated start: forget the backoff so a human can recover a looped agent.
pub fn clear_crash_backoff(pubkey: &str) {
    if let Ok(mut watches) = crash_watches().lock() {
        watches.remove(&watch_key(pubkey));
    }
}

/// Record an unexpected harness exit. Returns the new instant-death count
/// when this exit was inside the window of an automatic spawn.
pub fn note_unexpected_exit(pubkey: &str) -> u32 {
    let Ok(mut watches) = crash_watches().lock() else {
        return 0;
    };
    let watch = watches.entry(watch_key(pubkey)).or_default();
    let Some(spawned_at) = watch.auto_spawned_at.take() else {
        return watch.consecutive_instant_deaths;
    };
    if spawned_at.elapsed() <= INSTANT_DEATH_WINDOW {
        watch.consecutive_instant_deaths = watch.consecutive_instant_deaths.saturating_add(1);
    } else {
        watch.consecutive_instant_deaths = 0;
    }
    watch.consecutive_instant_deaths
}

pub fn crash_loop_blocked(pubkey: &str) -> bool {
    crash_watches()
        .lock()
        .ok()
        .and_then(|watches| {
            watches
                .get(&watch_key(pubkey))
                .map(|watch| watch.consecutive_instant_deaths >= CRASH_LOOP_CAP)
        })
        .unwrap_or(false)
}

pub fn crash_loop_error(_pubkey: &str) -> String {
    format!(
        "harness crash-looped {CRASH_LOOP_CAP} times within {INSTANT_DEATH_WINDOW:?} of automatic spawn; owner must start it"
    )
}

fn eq_hex(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

/// Pre-spawn author gate mirroring buzz-acp `author_allowed`.
///
/// Sibling verification is the local managed-agent set (same Desktop), not
/// the harness's live NIP-OA cache — we do not spawn a doomed process just
/// to ask it. Fail closed on unknown channel type by treating it as a DM.
pub fn author_passes_respond_to(
    record: &ManagedAgentRecord,
    author_pubkey: &str,
    owner_hex: Option<&str>,
    sibling_pubkeys: &HashSet<String>,
    is_dm: bool,
) -> bool {
    if eq_hex(&record.pubkey, author_pubkey) {
        return false;
    }
    let owner_or_sibling = owner_hex.is_some_and(|owner| eq_hex(owner, author_pubkey))
        || sibling_pubkeys
            .iter()
            .any(|sibling| eq_hex(sibling, author_pubkey));
    if is_dm {
        return owner_or_sibling;
    }
    match record.respond_to {
        RespondTo::Anyone => true,
        RespondTo::OwnerOnly => owner_or_sibling,
        RespondTo::Allowlist => {
            owner_or_sibling
                || record
                    .respond_to_allowlist
                    .iter()
                    .any(|entry| eq_hex(entry, author_pubkey))
        }
    }
}

pub fn decide_inbound_respawn(
    record: &ManagedAgentRecord,
    author_pubkey: &str,
    owner_hex: Option<&str>,
    sibling_pubkeys: &HashSet<String>,
    is_running: bool,
    is_dm: bool,
) -> InboundRespawnDecision {
    if !record.is_active {
        return InboundRespawnDecision::SkipInactive;
    }
    if !matches!(record.backend, super::BackendKind::Local) {
        return InboundRespawnDecision::SkipNotLocal;
    }
    if eq_hex(&record.pubkey, author_pubkey) {
        return InboundRespawnDecision::SkipSelfAuthored;
    }
    if is_running {
        return InboundRespawnDecision::SkipAlreadyRunning;
    }
    if !author_passes_respond_to(record, author_pubkey, owner_hex, sibling_pubkeys, is_dm) {
        return InboundRespawnDecision::SkipAuthorGated;
    }
    if crash_loop_blocked(&record.pubkey) {
        return InboundRespawnDecision::SkipCrashBackoff;
    }
    InboundRespawnDecision::Spawn
}

pub fn decide_apply_restart(record: &ManagedAgentRecord, is_running: bool) -> ApplyRestartDecision {
    if !record.is_active {
        return ApplyRestartDecision::SkipInactive;
    }
    if !matches!(record.backend, super::BackendKind::Local) {
        return ApplyRestartDecision::SkipNotLocal;
    }
    if is_running {
        return ApplyRestartDecision::SkipAlreadyRunning;
    }
    if crash_loop_blocked(&record.pubkey) {
        return ApplyRestartDecision::SkipCrashBackoff;
    }
    ApplyRestartDecision::Start
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::BackendKind;

    fn record() -> ManagedAgentRecord {
        let mut record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
            "pubkey": "aa".repeat(32),
            "name": "Probe",
            "relay_url": "",
            "acp_command": "",
            "agent_command": "",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 0,
            "system_prompt": null,
            "created_at": "",
            "updated_at": "",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }))
        .unwrap();
        record.backend = BackendKind::Local;
        record.is_active = true;
        record.respond_to = RespondTo::OwnerOnly;
        record
    }

    fn owner() -> String {
        "bb".repeat(32)
    }

    fn stranger() -> String {
        "cc".repeat(32)
    }

    fn sibling() -> String {
        "dd".repeat(32)
    }

    fn siblings() -> HashSet<String> {
        HashSet::from([sibling()])
    }

    #[test]
    fn inbound_spawns_stopped_active_owner_mention() {
        reset_crash_watches_for_tests();
        let decision = decide_inbound_respawn(
            &record(),
            &owner(),
            Some(&owner()),
            &siblings(),
            false,
            false,
        );
        assert_eq!(decision, InboundRespawnDecision::Spawn);
    }

    #[test]
    fn inbound_skips_archived() {
        reset_crash_watches_for_tests();
        let mut archived = record();
        archived.is_active = false;
        let decision = decide_inbound_respawn(
            &archived,
            &owner(),
            Some(&owner()),
            &siblings(),
            false,
            false,
        );
        assert_eq!(decision, InboundRespawnDecision::SkipInactive);
    }

    #[test]
    fn inbound_skips_provider_backend() {
        reset_crash_watches_for_tests();
        let mut remote = record();
        remote.backend = BackendKind::Provider {
            id: "remote".into(),
            config: serde_json::json!({}),
        };
        let decision =
            decide_inbound_respawn(&remote, &owner(), Some(&owner()), &siblings(), false, false);
        assert_eq!(decision, InboundRespawnDecision::SkipNotLocal);
    }

    #[test]
    fn inbound_skips_already_running() {
        reset_crash_watches_for_tests();
        let decision = decide_inbound_respawn(
            &record(),
            &owner(),
            Some(&owner()),
            &siblings(),
            true,
            false,
        );
        assert_eq!(decision, InboundRespawnDecision::SkipAlreadyRunning);
    }

    #[test]
    fn inbound_skips_self_authored() {
        reset_crash_watches_for_tests();
        let agent = record();
        let decision = decide_inbound_respawn(
            &agent,
            &agent.pubkey,
            Some(&owner()),
            &siblings(),
            false,
            false,
        );
        assert_eq!(decision, InboundRespawnDecision::SkipSelfAuthored);
    }

    #[test]
    fn inbound_owner_only_drops_stranger() {
        reset_crash_watches_for_tests();
        let decision = decide_inbound_respawn(
            &record(),
            &stranger(),
            Some(&owner()),
            &siblings(),
            false,
            false,
        );
        assert_eq!(decision, InboundRespawnDecision::SkipAuthorGated);
    }

    #[test]
    fn inbound_owner_only_accepts_sibling() {
        reset_crash_watches_for_tests();
        let decision = decide_inbound_respawn(
            &record(),
            &sibling(),
            Some(&owner()),
            &siblings(),
            false,
            false,
        );
        assert_eq!(decision, InboundRespawnDecision::Spawn);
    }

    #[test]
    fn inbound_allowlist_accepts_listed_stranger() {
        reset_crash_watches_for_tests();
        let mut listed = record();
        listed.respond_to = RespondTo::Allowlist;
        listed.respond_to_allowlist = vec![stranger()];
        let decision = decide_inbound_respawn(
            &listed,
            &stranger(),
            Some(&owner()),
            &siblings(),
            false,
            false,
        );
        assert_eq!(decision, InboundRespawnDecision::Spawn);
    }

    #[test]
    fn inbound_dm_ignores_anyone_and_allowlist() {
        reset_crash_watches_for_tests();
        let mut anyone = record();
        anyone.respond_to = RespondTo::Anyone;
        let decision = decide_inbound_respawn(
            &anyone,
            &stranger(),
            Some(&owner()),
            &siblings(),
            false,
            true,
        );
        assert_eq!(decision, InboundRespawnDecision::SkipAuthorGated);
    }

    #[test]
    fn inbound_anyone_accepts_stranger_in_channel() {
        reset_crash_watches_for_tests();
        let mut anyone = record();
        anyone.respond_to = RespondTo::Anyone;
        let decision = decide_inbound_respawn(
            &anyone,
            &stranger(),
            Some(&owner()),
            &siblings(),
            false,
            false,
        );
        assert_eq!(decision, InboundRespawnDecision::Spawn);
    }

    #[test]
    fn crash_loop_blocks_after_cap() {
        reset_crash_watches_for_tests();
        let pubkey = record().pubkey;
        for _ in 0..CRASH_LOOP_CAP {
            note_automatic_spawn(&pubkey);
            assert!(note_unexpected_exit(&pubkey) > 0);
        }
        assert!(crash_loop_blocked(&pubkey));
        let decision = decide_inbound_respawn(
            &record(),
            &owner(),
            Some(&owner()),
            &siblings(),
            false,
            false,
        );
        assert_eq!(decision, InboundRespawnDecision::SkipCrashBackoff);
        assert_eq!(
            decide_apply_restart(&record(), false),
            ApplyRestartDecision::SkipCrashBackoff
        );
    }

    #[test]
    fn owner_clear_recovers_crash_loop() {
        reset_crash_watches_for_tests();
        let pubkey = record().pubkey;
        for _ in 0..CRASH_LOOP_CAP {
            note_automatic_spawn(&pubkey);
            note_unexpected_exit(&pubkey);
        }
        clear_crash_backoff(&pubkey);
        assert!(!crash_loop_blocked(&pubkey));
        assert_eq!(
            decide_apply_restart(&record(), false),
            ApplyRestartDecision::Start
        );
    }

    #[test]
    fn apply_restart_starts_stopped_active() {
        reset_crash_watches_for_tests();
        assert_eq!(
            decide_apply_restart(&record(), false),
            ApplyRestartDecision::Start
        );
    }

    #[test]
    fn apply_restart_skips_archived() {
        reset_crash_watches_for_tests();
        let mut archived = record();
        archived.is_active = false;
        assert_eq!(
            decide_apply_restart(&archived, false),
            ApplyRestartDecision::SkipInactive
        );
    }

    #[test]
    fn unexpected_exit_without_auto_spawn_does_not_count() {
        reset_crash_watches_for_tests();
        let pubkey = record().pubkey;
        assert_eq!(note_unexpected_exit(&pubkey), 0);
        assert!(!crash_loop_blocked(&pubkey));
    }
}
