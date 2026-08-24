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
///
/// Both families paginate backward from the newest event with the bridge's
/// composite `(until, before_id)` cursor (B4): the relay clamps limitless
/// filters to the newest 1000 rows, so a single unpaginated refresh against a
/// >1000-event stream silently gaps everything older.
async fn fetch_new_events(
    config: &DispatcherConfig,
    relay: &RelayHttp,
    since: u64,
) -> Result<Vec<Event>, DispatchError> {
    let mut events = Vec::new();

    // Tracker stream: roots, revisions, comments, dispatch actions.
    events.extend(query_paginated_events(relay, tracker_refresh_filter(config, since)).await?);

    // Monitored-channel pin reactions for capture.
    if !config.monitored_channels.is_empty() {
        events.extend(
            query_paginated_events(
                relay,
                reaction_refresh_filter(&config.monitored_channels, since),
            )
            .await?,
        );
    }

    Ok(events)
}

/// Tracker-stream inclusive-refresh filter. MUST serialize as a single filter
/// OBJECT: the bridge parses the `POST /query` body as `Vec<Filter>` (B1).
fn tracker_refresh_filter(config: &DispatcherConfig, since: u64) -> Value {
    json!({
        "kinds": [
            buzz_sdk::kind::KIND_TRACKER_ISSUE,
            buzz_sdk::kind::KIND_TRACKER_REVISION,
            buzz_sdk::kind::KIND_TRACKER_COMMENT,
            buzz_sdk::kind::KIND_TRACKER_DISPATCH_ACTION,
        ],
        "#h": [config.stream_id.to_string()],
        "since": since,
    })
}

/// Monitored-channel reaction inclusive-refresh filter (single filter OBJECT;
/// B1).
fn reaction_refresh_filter(channels: &[Uuid], since: u64) -> Value {
    let channel_ids: Vec<String> = channels.iter().map(Uuid::to_string).collect();
    json!({ "kinds": [7], "#h": channel_ids, "since": since })
}

/// Page size for backward refresh pagination. Below the relay's 1000-row page
/// clamp; matches the buzz-cli `QUERY_PAGE_SIZE`.
const REFRESH_PAGE_SIZE: usize = 500;
/// Hard cap on pages per refresh so a pathological stream cannot spin the
/// tick forever (500 * 200 = 100k events per family per tick).
const REFRESH_MAX_PAGES: usize = 200;

/// Query one filter backward across bounded pages until the result is
/// exhausted, newest page first. The base filter carries the inclusive
/// `since`; each full page advances the composite `(until, before_id)`
/// cursor, exactly the shape `buzz-cli query_pages` uses.
async fn query_paginated_events(
    relay: &RelayHttp,
    mut filter: Value,
) -> Result<Vec<Event>, DispatchError> {
    let mut events = Vec::new();
    for _ in 0..REFRESH_MAX_PAGES {
        filter["limit"] = json!(REFRESH_PAGE_SIZE);
        let page = query_page(relay, &filter).await?;
        let done = page.len() < REFRESH_PAGE_SIZE;
        if !done {
            advance_refresh_cursor(&mut filter, &page)?;
        }
        events.extend(page);
        if done {
            return Ok(events);
        }
    }
    Err(DispatchError::Other(format!(
        "refresh pagination exceeded {REFRESH_MAX_PAGES} pages; aborting instead of looping"
    )))
}

/// One `POST /query` page; unparseable events are skipped with a warning.
async fn query_page(relay: &RelayHttp, filter: &Value) -> Result<Vec<Event>, DispatchError> {
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

/// Advance a backward pagination cursor past `page`'s oldest event. The
/// bridge orders pages `(created_at DESC, id ASC)` and keys the next page on
/// `created_at < until OR (created_at = until AND id > before_id)`, so both
/// fields must move together (`before_id` without `until` is a 400).
fn advance_refresh_cursor(filter: &mut Value, page: &[Event]) -> Result<(), DispatchError> {
    let last = page.last().ok_or_else(|| {
        DispatchError::Other("cannot advance refresh cursor on an empty page".into())
    })?;
    filter["until"] = json!(last.created_at.as_secs());
    filter["before_id"] = json!(last.id.to_hex());
    Ok(())
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
    use crate::config::{DecisionSurface, LivenessConfig};
    use nostr::{EventBuilder, Keys, Kind, Timestamp};

    fn test_config() -> DispatcherConfig {
        let home = Uuid::new_v4();
        DispatcherConfig {
            relay_url: Some("http://localhost:3000".to_string()),
            service_id: "triage-dispatcher".to_string(),
            stream_id: Uuid::new_v4(),
            poll_interval_secs: 30,
            monitored_channels: vec![home],
            pin_emoji: "\u{1F4CC}".to_string(),
            default_channel: Some(home),
            routing: Vec::new(),
            thresholds: Vec::new(),
            roles: std::collections::BTreeMap::new(),
            decision_surface: DecisionSurface { channel: home },
            liveness: LivenessConfig {
                channel: home,
                owner: None,
                stale_after_secs: 300,
            },
            state_dir: std::path::PathBuf::from("/tmp/dispatcher-state-test"),
        }
    }

    #[test]
    fn current_unix_seconds_is_reasonable() {
        let now = current_unix_seconds();
        assert!(now > 1_700_000_000, "epoch seconds should be post-2023");
    }

    /// B1 regression guard: refresh filters are single filter OBJECTS. The
    /// bridge parses the `POST /query` body as `Vec<Filter>`, so wrapping a
    /// filter in its own array double-nests the body and 400s every query.
    #[test]
    fn refresh_filters_are_objects_not_arrays() {
        let config = test_config();
        let tracker = tracker_refresh_filter(&config, 42);
        assert!(tracker.is_object(), "tracker filter must be an object");
        assert_eq!(tracker["since"], json!(42));
        assert_eq!(tracker["#h"], json!([config.stream_id.to_string()]));
        assert!(tracker["kinds"].as_array().expect("kinds").len() >= 4);

        let reactions = reaction_refresh_filter(&config.monitored_channels, 42);
        assert!(reactions.is_object(), "reaction filter must be an object");
        assert_eq!(reactions["kinds"], json!([7]));
        assert_eq!(reactions["since"], json!(42));

        // And the transport serializes them as a flat top-level array.
        let body = crate::relay_http::query_body(std::slice::from_ref(&tracker)).expect("body");
        let parsed: Value = serde_json::from_slice(&body).expect("json");
        assert!(
            parsed[0].is_object(),
            "body must be [{{...}}], not [[{{...}}]]"
        );
    }

    #[test]
    fn advance_refresh_cursor_sets_composite_until_before_id() {
        let keys = Keys::generate();
        let newer = EventBuilder::new(Kind::TextNote, "newer")
            .custom_created_at(Timestamp::from(200))
            .sign_with_keys(&keys)
            .expect("signed");
        let older = EventBuilder::new(Kind::TextNote, "older")
            .custom_created_at(Timestamp::from(100))
            .sign_with_keys(&keys)
            .expect("signed");

        // Pages arrive newest-first; the cursor keys off the page's LAST
        // (oldest) event.
        let page = vec![newer, older.clone()];
        let mut filter = json!({ "kinds": [1], "since": 0 });
        advance_refresh_cursor(&mut filter, &page).expect("advance");
        assert_eq!(filter["until"], json!(100));
        assert_eq!(filter["before_id"], json!(older.id.to_hex()));
        assert_eq!(filter["since"], json!(0), "since stays inclusive");
    }

    #[test]
    fn advance_refresh_cursor_rejects_empty_page() {
        let mut filter = json!({ "kinds": [1] });
        assert!(advance_refresh_cursor(&mut filter, &[]).is_err());
    }
}
