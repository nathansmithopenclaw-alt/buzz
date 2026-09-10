//! External heartbeat watchdog (plan M17, binding amendment 2).
//!
//! Runs under its OWN identity (separate launchd job): reads the latest
//! kind-40024 heartbeat per `(stream, signer, service_id)` and, when one goes
//! stale beyond `liveness.stale_after_secs`, posts a channel message naming
//! the config liveness owner. Only heartbeats signed by a key in
//! `liveness.signer_allowlist` count (M6): any channel member can publish a
//! 40024 carrying the service slug, so the SIGNER — not the content — is the
//! proof of life. Surface is message-only — never a shared-canvas
//! mutation (canvases are hand-curated; bot read-modify-write is a clobber
//! class). A dead process cannot author its own death notice; watchdog death
//! is covered by the next layer up (disk-guard pattern).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use buzz_sdk::tracker::parse_service_heartbeat;
use buzz_sdk::{build_message, kind};
use nostr::Event;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, error, info, warn};

use crate::config::DispatcherConfig;
use crate::error::DispatchError;
use crate::projection::{InstanceLock, WATCHDOG_LOCK_FILE};
use crate::relay_http::{parse_submit_receipt, RelayHttp};

/// Latest observed heartbeat for one `(signer, service_id)` pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestHeartbeat {
    /// Signing pubkey (hex).
    pub signer: String,
    /// Service slug from the `d` tag.
    pub service_id: String,
    /// Newest `observed_at` seen (unix seconds).
    pub observed_at: u64,
}

/// A service whose newest heartbeat is older than the staleness window (or
/// that was never seen).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleService {
    /// Signing pubkey (hex).
    pub signer: String,
    /// Service slug.
    pub service_id: String,
    /// Newest heartbeat observed, if any.
    pub last_seen: Option<u64>,
}

/// Pure staleness decision: given the latest heartbeat per service key and
/// `now`, report every tracked service at/over the staleness window. Services
/// present in `expected` but absent from `latest` are stale (never seen).
pub fn stale_services(
    expected: &[(String, String)],
    latest: &BTreeMap<(String, String), LatestHeartbeat>,
    now: u64,
    stale_after_secs: u64,
) -> Vec<StaleService> {
    let mut stale = Vec::new();
    for (signer, service_id) in expected {
        let key = (signer.clone(), service_id.clone());
        match latest.get(&key) {
            Some(heartbeat) if now.saturating_sub(heartbeat.observed_at) > stale_after_secs => {
                stale.push(StaleService {
                    signer: signer.clone(),
                    service_id: service_id.clone(),
                    last_seen: Some(heartbeat.observed_at),
                });
            }
            Some(_) => {}
            None => stale.push(StaleService {
                signer: signer.clone(),
                service_id: service_id.clone(),
                last_seen: None,
            }),
        }
    }
    stale
}

/// Reduce a batch of raw heartbeat events to the latest per
/// `(signer, service_id)`.
pub fn latest_heartbeats(
    events: &[Event],
    stream_id: uuid::Uuid,
    now: u64,
) -> BTreeMap<(String, String), LatestHeartbeat> {
    let mut latest: BTreeMap<(String, String), LatestHeartbeat> = BTreeMap::new();
    for event in events {
        let Ok(heartbeat) = parse_service_heartbeat(event, now) else {
            continue;
        };
        if heartbeat.stream_id != stream_id {
            continue;
        }
        let key = (
            heartbeat.author_pubkey.clone(),
            heartbeat.heartbeat.service_id.clone(),
        );
        let entry = latest.entry(key).or_insert(LatestHeartbeat {
            signer: heartbeat.author_pubkey.clone(),
            service_id: heartbeat.heartbeat.service_id.clone(),
            observed_at: 0,
        });
        if heartbeat.heartbeat.observed_at > entry.observed_at {
            entry.observed_at = heartbeat.heartbeat.observed_at;
        }
    }
    latest
}

/// Heartbeat query filter for one watchdog tick. MUST serialize as a single
/// filter OBJECT: the bridge parses the `POST /query` body as `Vec<Filter>`
/// (B1 regression guard — see the unit test). `authors` scopes the relay
/// query to the allowlisted signers (M6); the allowlist check is applied
/// again in code when reducing the results.
fn heartbeat_filter(config: &DispatcherConfig) -> serde_json::Value {
    json!({
        "kinds": [kind::KIND_TRACKER_SERVICE_HEARTBEAT],
        "#h": [config.stream_id.to_string()],
        "authors": config.liveness.signer_allowlist,
        "limit": 200,
    })
}

/// Newest `observed_at` for `service_id` among ALLOWLISTED signers only
/// (M6). A 40024 with the service slug signed by anyone else — any channel
/// member can build one — never moves the staleness clock.
pub fn best_allowed_observed_at(
    latest: &BTreeMap<(String, String), LatestHeartbeat>,
    service_id: &str,
    signer_allowlist: &[String],
) -> Option<u64> {
    latest
        .iter()
        .filter(|((signer, service), _)| {
            service == service_id && signer_allowlist.iter().any(|allowed| allowed == signer)
        })
        .map(|(_, heartbeat)| heartbeat.observed_at)
        .max()
}

/// Watchdog state for stale-episode edge detection. Persisted in
/// `state_dir/watchdog_state.json`: it used to be in-memory only, so every
/// restart reset the never-seen grace window and re-posted the stale notice
/// mid-episode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Episode {
    /// When the watchdog first started monitoring (never-seen grace anchor).
    monitoring_since: u64,
    /// Whether the previous tick judged the service stale.
    was_stale: bool,
}

/// Persisted watchdog episode-state file inside the config `state_dir`.
pub const WATCHDOG_STATE_FILE: &str = "watchdog_state.json";

/// Load persisted episode state; a missing file yields a fresh episode.
/// A CORRUPT file is a hard error (crate-wide corrupt-state policy — never
/// silent reset). Returns `(episode, freshly_created)`.
fn load_episode(state_dir: &Path) -> Result<(Episode, bool), DispatchError> {
    let path = state_dir.join(WATCHDOG_STATE_FILE);
    if !path.exists() {
        return Ok((
            Episode {
                monitoring_since: current_unix_seconds(),
                was_stale: false,
            },
            true,
        ));
    }
    let raw = fs::read_to_string(&path)?;
    let episode = serde_json::from_str(&raw)
        .map_err(|error| DispatchError::Other(format!("corrupt watchdog state file: {error}")))?;
    Ok((episode, false))
}

fn persist_episode(state_dir: &Path, episode: &Episode) -> Result<(), DispatchError> {
    crate::projection::write_atomic(
        &state_dir.join(WATCHDOG_STATE_FILE),
        serde_json::to_string(episode)?.as_bytes(),
    )
}

/// Run the watchdog until the process is stopped. Watches the configured
/// `service_id`; ONLY heartbeats signed by a key in
/// `liveness.signer_allowlist` count (M6 — any channel member can publish a
/// 40024 with the service slug, so the signer is the proof of life). Service
/// key rotation is a config edit: update the allowlist in the same change.
pub async fn watch(config: DispatcherConfig, relay: RelayHttp) -> Result<(), DispatchError> {
    // M7: single-instance guard, matching the resident loop.
    let _lock = InstanceLock::acquire(&config.state_dir, WATCHDOG_LOCK_FILE)?;
    let interval = Duration::from_secs(config.poll_interval_secs);
    let (mut episode, created) = load_episode(&config.state_dir)?;
    if created {
        // Anchor the never-seen grace durably so restarts cannot extend it.
        persist_episode(&config.state_dir, &episode)?;
    }

    info!(
        service_id = %config.service_id,
        stale_after_secs = config.liveness.stale_after_secs,
        resumed_stale = episode.was_stale,
        "watchdog starting"
    );

    loop {
        match tick_watchdog(&config, &relay, &mut episode).await {
            Ok(()) => {}
            Err(error) => error!(%error, "watchdog tick failed"),
        }
        tokio::time::sleep(interval).await;
    }
}

async fn tick_watchdog(
    config: &DispatcherConfig,
    relay: &RelayHttp,
    episode: &mut Episode,
) -> Result<(), DispatchError> {
    let now = current_unix_seconds();
    let filter = heartbeat_filter(config);
    let values = relay.query(std::slice::from_ref(&filter)).await?;
    let events: Vec<Event> = values
        .into_iter()
        .filter_map(|value| serde_json::from_value::<Event>(value).ok())
        .collect();

    let latest = latest_heartbeats(&events, config.stream_id, now);

    // M6: only allowlisted signers count; newest heartbeat wins. A spoofed
    // 40024 from any other member never updates the staleness clock.
    let service_best = best_allowed_observed_at(
        &latest,
        &config.service_id,
        &config.liveness.signer_allowlist,
    );

    // Never-seen gets a grace window from watchdog start so a fresh deploy
    // does not page before the dispatcher emits its first heartbeat.
    let stale = match service_best {
        None => now.saturating_sub(episode.monitoring_since) > config.liveness.stale_after_secs,
        Some(observed_at) => now.saturating_sub(observed_at) > config.liveness.stale_after_secs,
    };

    // One notice per stale episode (healthy -> stale transition only).
    if stale && !episode.was_stale {
        notify_stale(
            config,
            relay,
            &StaleService {
                signer: String::new(),
                service_id: config.service_id.clone(),
                last_seen: service_best,
            },
        )
        .await?;
    }
    if !stale && episode.was_stale {
        info!(service_id = %config.service_id, "service heartbeat recovered");
    }
    if episode.was_stale != stale {
        episode.was_stale = stale;
        // Persist the edge so a restart mid-episode neither re-notifies nor
        // loses the recovery transition.
        persist_episode(&config.state_dir, episode)?;
    }
    if stale {
        debug!(service_id = %config.service_id, "service stale");
    }
    Ok(())
}

/// Message-only stale notice (binding amendment 2: never a canvas mutation).
async fn notify_stale(
    config: &DispatcherConfig,
    relay: &RelayHttp,
    stale: &StaleService,
) -> Result<(), DispatchError> {
    let detail = match stale.last_seen {
        Some(observed_at) => format!("last heartbeat at unix {observed_at}"),
        None => "no heartbeat ever observed".to_string(),
    };
    let content = format!(
        "⚠️ Dispatcher watchdog: service `{}` is stale ({detail}); stale window {}s.",
        stale.service_id, config.liveness.stale_after_secs
    );
    let mentions: Vec<&str> = config
        .liveness
        .owner
        .as_deref()
        .map(|owner| vec![owner])
        .unwrap_or_default();
    let builder = build_message(
        config.liveness.channel,
        &content,
        None,
        &mentions,
        false,
        &[],
        &[],
    )?;
    let event = builder
        .sign_with_keys(relay.keys())
        .map_err(|error| DispatchError::Identity(format!("signing failed: {error}")))?;
    let raw = relay.submit_event(&event).await?;
    let receipt = parse_submit_receipt(&raw)?;
    if !receipt.accepted {
        return Err(DispatchError::Relay(format!(
            "stale notice rejected: {}",
            receipt.message
        )));
    }
    warn!(
        service_id = %stale.service_id,
        channel = %config.liveness.channel,
        "stale-service notice posted"
    );
    Ok(())
}

fn current_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_sdk::tracker::{
        build_service_heartbeat, ServiceHealth, ServiceHealthStatus, ServiceHeartbeat,
        TRACKER_VERSION,
    };
    use nostr::Keys;

    fn heartbeat_event(
        stream_id: uuid::Uuid,
        service_id: &str,
        observed_at: u64,
        keys: &Keys,
    ) -> Event {
        let heartbeat = ServiceHeartbeat {
            v: TRACKER_VERSION,
            service_id: service_id.to_string(),
            instance_id: uuid::Uuid::new_v4(),
            observed_at,
            cursor: String::new(),
            health: ServiceHealth {
                status: ServiceHealthStatus::Healthy,
                detail: None,
            },
        };
        build_service_heartbeat(stream_id, &heartbeat)
            .expect("built")
            .custom_created_at(nostr::Timestamp::from(observed_at))
            .sign_with_keys(keys)
            .expect("signed")
    }

    #[test]
    fn staleness_window_decision() {
        let signer = "a".repeat(64);
        let service = "triage-dispatcher".to_string();
        let mut latest = BTreeMap::new();
        latest.insert(
            (signer.clone(), service.clone()),
            LatestHeartbeat {
                signer: signer.clone(),
                service_id: service.clone(),
                observed_at: 1_000,
            },
        );

        let expected = vec![(signer.clone(), service.clone())];

        // Inside the window: healthy.
        assert!(stale_services(&expected, &latest, 1_000 + 300, 300).is_empty());
        // Over the window: stale with last_seen.
        let stale = stale_services(&expected, &latest, 1_301, 300);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].last_seen, Some(1_000));
        // Never-seen service is stale with no last_seen.
        let missing = vec![("b".repeat(64), "other".to_string())];
        let stale = stale_services(&missing, &latest, 1_301, 300);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].last_seen, None);
    }

    #[test]
    fn latest_heartbeats_keeps_newest_per_signer_service_and_stream() {
        let stream = uuid::Uuid::new_v4();
        let other_stream = uuid::Uuid::new_v4();
        let keys = Keys::generate();

        let events = vec![
            heartbeat_event(stream, "svc", 100, &keys),
            heartbeat_event(stream, "svc", 200, &keys),
            heartbeat_event(other_stream, "svc", 999, &keys),
        ];
        let latest = latest_heartbeats(&events, stream, 1_000);
        assert_eq!(latest.len(), 1);
        let entry = latest
            .get(&(keys.public_key().to_hex(), "svc".to_string()))
            .expect("entry");
        assert_eq!(entry.observed_at, 200, "newest wins, other stream ignored");
    }

    /// B1 regression guard: the watchdog filter is a single filter OBJECT,
    /// not an array-wrapped filter (the bridge parses `Vec<Filter>`).
    #[test]
    fn heartbeat_filter_is_an_object_not_an_array() {
        let home = uuid::Uuid::new_v4();
        let config = crate::config::DispatcherConfig {
            remote_controls: false,
            relay_url: Some("http://localhost:3000".to_string()),
            service_id: "triage-dispatcher".to_string(),
            stream_id: uuid::Uuid::new_v4(),
            poll_interval_secs: 30,
            monitored_channels: Vec::new(),
            pin_emoji: "\u{1F4CC}".to_string(),
            default_channel: None,
            routing: Vec::new(),
            thresholds: Vec::new(),
            roles: std::collections::BTreeMap::new(),
            decision_surface: crate::config::DecisionSurface { channel: home },
            liveness: crate::config::LivenessConfig {
                channel: home,
                owner: None,
                stale_after_secs: 300,
                signer_allowlist: vec!["d".repeat(64)],
            },
            state_dir: std::path::PathBuf::from("/tmp/watchdog-state-test"),
        };
        let filter = heartbeat_filter(&config);
        assert!(filter.is_object(), "filter must be an object");
        assert_eq!(filter["#h"], json!([config.stream_id.to_string()]));
        assert_eq!(filter["authors"], json!(config.liveness.signer_allowlist));
        assert_eq!(filter["limit"], json!(200));
        let body = crate::relay_http::query_body(std::slice::from_ref(&filter)).expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert!(
            parsed[0].is_object(),
            "body must be [{{...}}], not [[{{...}}]]"
        );
    }

    /// M6 regression guard: a 40024 with the service slug signed by a key
    /// OUTSIDE the allowlist (spoofing by any channel member) never moves
    /// the staleness clock; only allowlisted signers count.
    #[test]
    fn spoofed_heartbeats_outside_allowlist_do_not_move_the_clock() {
        let stream = uuid::Uuid::new_v4();
        let service = "triage-dispatcher".to_string();
        let service_keys = Keys::generate();
        let spoofer_keys = Keys::generate();

        let events = vec![
            heartbeat_event(stream, &service, 1_000, &service_keys),
            heartbeat_event(stream, &service, 9_999, &spoofer_keys),
        ];
        let latest = latest_heartbeats(&events, stream, 10_000);

        // Only the service key is allowlisted.
        let allowlist = vec![service_keys.public_key().to_hex()];
        assert_eq!(
            best_allowed_observed_at(&latest, &service, &allowlist),
            Some(1_000),
            "spoofer's newer heartbeat must not count"
        );

        // Empty allowlist trusts nothing, even the real service key.
        assert_eq!(best_allowed_observed_at(&latest, &service, &[]), None);

        // A different service slug is ignored too.
        assert_eq!(
            best_allowed_observed_at(&latest, "other-service", &allowlist),
            None
        );
    }

    /// Minor fix: episode state survives restarts (previously in-memory
    /// only, so a restart mid-episode re-posted the stale notice and reset
    /// the never-seen grace).
    #[test]
    fn episode_state_round_trips_through_restart() {
        let dir = tempfile::tempdir().expect("tempdir");

        let (fresh, created) = load_episode(dir.path()).expect("fresh load");
        assert!(created);
        assert!(!fresh.was_stale);
        persist_episode(dir.path(), &fresh).expect("anchor");

        // Mid-episode transition must persist.
        let mut stale = fresh.clone();
        stale.was_stale = true;
        persist_episode(dir.path(), &stale).expect("persist edge");

        let (loaded, created) = load_episode(dir.path()).expect("reload");
        assert!(!created, "restart resumes, does not reset");
        assert_eq!(loaded, stale);
        assert_eq!(loaded.monitoring_since, fresh.monitoring_since);
    }

    #[test]
    fn corrupt_episode_state_is_a_hard_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(WATCHDOG_STATE_FILE), "{nope").expect("write");
        assert!(load_episode(dir.path()).is_err());
    }
}
