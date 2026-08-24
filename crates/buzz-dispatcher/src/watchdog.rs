//! External heartbeat watchdog (plan M17, binding amendment 2).
//!
//! Runs under its OWN identity (separate launchd job): reads the latest
//! kind-40024 heartbeat per `(stream, signer, service_id)` and, when one goes
//! stale beyond `liveness.stale_after_secs`, posts a channel message naming
//! the config liveness owner. Surface is message-only — never a shared-canvas
//! mutation (canvases are hand-curated; bot read-modify-write is a clobber
//! class). A dead process cannot author its own death notice; watchdog death
//! is covered by the next layer up (disk-guard pattern).

use std::collections::BTreeMap;
use std::time::Duration;

use buzz_sdk::tracker::parse_service_heartbeat;
use buzz_sdk::{build_message, kind};
use nostr::Event;
use serde_json::json;
use tracing::{debug, error, info, warn};

use crate::config::DispatcherConfig;
use crate::error::DispatchError;
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
/// (B1 regression guard — see the unit test).
fn heartbeat_filter(config: &DispatcherConfig) -> serde_json::Value {
    json!({
        "kinds": [kind::KIND_TRACKER_SERVICE_HEARTBEAT],
        "#h": [config.stream_id.to_string()],
        "limit": 200,
    })
}

/// Watchdog process state for stale-episode edge detection.
#[derive(Debug, Clone)]
struct Episode {
    /// When this watchdog process started monitoring (never-seen grace).
    monitoring_since: u64,
    /// Whether the previous tick judged the service stale.
    was_stale: bool,
}

/// Run the watchdog until the process is stopped. Watches the configured
/// `service_id`; the signer is not pinned in config (a service-key rotation
/// must not blind the watchdog), so any signer for that service counts and is
/// reported when stale.
pub async fn watch(config: DispatcherConfig, relay: RelayHttp) -> Result<(), DispatchError> {
    let interval = Duration::from_secs(config.poll_interval_secs);
    let mut episode = Episode {
        monitoring_since: current_unix_seconds(),
        was_stale: false,
    };

    info!(
        service_id = %config.service_id,
        stale_after_secs = config.liveness.stale_after_secs,
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

    // Any signer for the configured service counts; newest heartbeat wins.
    let service_best: Option<u64> = latest
        .iter()
        .filter(|((_, service_id), _)| service_id == &config.service_id)
        .map(|(_, heartbeat)| heartbeat.observed_at)
        .max();

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
    episode.was_stale = stale;
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
            },
            state_dir: std::path::PathBuf::from("/tmp/watchdog-state-test"),
        };
        let filter = heartbeat_filter(&config);
        assert!(filter.is_object(), "filter must be an object");
        assert_eq!(filter["#h"], json!([config.stream_id.to_string()]));
        assert_eq!(filter["limit"], json!(200));
        let body = crate::relay_http::query_body(std::slice::from_ref(&filter)).expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert!(
            parsed[0].is_object(),
            "body must be [{{...}}], not [[{{...}}]]"
        );
    }
}
