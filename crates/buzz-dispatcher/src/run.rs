//! Resident capture/route/escalate/heartbeat loop.
//!
//! Each tick: refresh the projection with an inclusive cursor query, fold the
//! tracker stream with workspace authorization, plan deterministic actions,
//! emit them (marking the fired ledger), then publish a heartbeat carrying the
//! advanced cursor. The loop is idempotent: request-ID pre-checks make any
//! replayed or duplicate delivery a no-op, and a restart resumes from the
//! persisted cursor.

use std::time::Duration;

use buzz_sdk::tracker::{fold_tracker_events, parse_tracker_event, TrackerFold};
use nostr::Event;
use serde_json::{json, Value};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::config::DispatcherConfig;
use crate::emit::Emitter;
use crate::error::DispatchError;
use crate::projection::ProjectionStore;
use crate::relay_http::RelayHttp;
use crate::rules;

/// Run the dispatcher until the process is stopped.
pub async fn run(
    config: DispatcherConfig,
    relay: RelayHttp,
    instance_id: Uuid,
) -> Result<(), DispatchError> {
    let mut store = ProjectionStore::open(&config.state_dir)?;
    let emitter = Emitter::new(&relay, &config, instance_id);
    let authorization = config.authorization(&relay.pubkey_hex());
    let interval = Duration::from_secs(config.poll_interval_secs);

    info!(
        service_id = %config.service_id,
        pubkey = %relay.pubkey_hex(),
        stream = %config.stream_id,
        since = store.since(),
        "dispatcher loop starting"
    );

    loop {
        if let Err(error) = tick(&config, &relay, &emitter, &authorization, &mut store).await {
            // Never crash the loop on a single bad tick; heartbeat liveness is
            // what surfaces a wedged dispatcher.
            error!(%error, "dispatcher tick failed");
        }
        tokio::time::sleep(interval).await;
    }
}

/// One poll cycle. Returns after emitting and committing.
pub async fn tick(
    config: &DispatcherConfig,
    relay: &RelayHttp,
    emitter: &Emitter<'_>,
    authorization: &buzz_sdk::tracker::TrackerAuthorization,
    store: &mut ProjectionStore,
) -> Result<(), DispatchError> {
    let since = store.since();

    // Inclusive refresh: query everything at/after the cursor. Event-id dedupe
    // in the store absorbs the boundary overlap.
    let fresh = fetch_new_events(config, relay, since).await?;
    let ingested = store.ingest(fresh);
    if ingested > 0 {
        debug!(count = ingested, "ingested new events");
    }

    let now = current_unix_seconds();
    let fold = fold_store(store, authorization, now);

    // Capture plans first (reactions), then fold-driven plans.
    let captures = rules::plan_captures(config, store);
    for plan in &captures {
        if let Err(error) = emitter.emit_capture(store, plan).await {
            warn!(%error, source = %plan.source_event_id, "capture emission failed");
        }
    }

    let actions = rules::plan_fold_actions(config, &fold, store, now);
    if !actions.is_empty() {
        if let Err(error) = emitter.emit_rule_actions(store, &actions).await {
            warn!(%error, count = actions.len(), "rule-action emission failed");
        }
    }

    store.commit()?;

    // Heartbeat last so its cursor reflects everything committed this tick.
    if let Err(error) = emitter.emit_heartbeat(store).await {
        warn!(%error, "heartbeat emission failed");
    }

    Ok(())
}

/// Fetch tracker-stream events, monitored-channel reactions, and prior dispatch
/// actions at/after `since`.
async fn fetch_new_events(
    config: &DispatcherConfig,
    relay: &RelayHttp,
    since: u64,
) -> Result<Vec<Event>, DispatchError> {
    let mut events = Vec::new();

    // Tracker stream: roots, revisions, comments, dispatch actions, heartbeats.
    let tracker_filter = json!([{
        "kinds": [
            buzz_sdk::kind::KIND_TRACKER_ISSUE,
            buzz_sdk::kind::KIND_TRACKER_REVISION,
            buzz_sdk::kind::KIND_TRACKER_COMMENT,
            buzz_sdk::kind::KIND_TRACKER_DISPATCH_ACTION,
        ],
        "#h": [config.stream_id.to_string()],
        "since": since,
    }]);
    events.extend(query_events(relay, &tracker_filter).await?);

    // Monitored-channel pin reactions for capture.
    if !config.monitored_channels.is_empty() {
        let channels: Vec<String> = config
            .monitored_channels
            .iter()
            .map(Uuid::to_string)
            .collect();
        let reaction_filter = json!([{ "kinds": [7], "#h": channels, "since": since }]);
        events.extend(query_events(relay, &reaction_filter).await?);
    }

    Ok(events)
}

async fn query_events(relay: &RelayHttp, filter: &Value) -> Result<Vec<Event>, DispatchError> {
    let values = relay.query(std::slice::from_ref(filter)).await?;
    let mut events = Vec::with_capacity(values.len());
    for value in values {
        match serde_json::from_value::<Event>(value) {
            Ok(event) => events.push(event),
            Err(error) => warn!(%error, "skipping unparseable query event"),
        }
    }
    Ok(events)
}

/// Fold the persisted journal into the current tracker projection. Non-tracker
/// events (reactions, heartbeats) fall out of the strict parser.
fn fold_store(
    store: &ProjectionStore,
    authorization: &buzz_sdk::tracker::TrackerAuthorization,
    now: u64,
) -> TrackerFold {
    let tracker_events = store
        .events()
        .iter()
        .filter_map(|event| parse_tracker_event(event, now).ok());
    fold_tracker_events(tracker_events, authorization)
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

    #[test]
    fn current_unix_seconds_is_reasonable() {
        let now = current_unix_seconds();
        assert!(now > 1_700_000_000, "epoch seconds should be post-2023");
    }
}
