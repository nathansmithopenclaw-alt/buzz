//! Resident capture/route/escalate/heartbeat loop.
//!
//! Each tick: replay any pending outbox publications, refresh the projection
//! with an inclusive cursor query, fold the tracker stream with workspace
//! authorization, plan deterministic actions, emit them (persist-before-
//! publish), then publish a heartbeat carrying the advanced cursor and this
//! tick's health. The loop is idempotent: request-ID pre-checks make any
//! replayed or duplicate delivery a no-op, and a restart resumes from the
//! persisted cursor + outbox.

use std::time::Duration;

use buzz_sdk::tracker::{
    fold_tracker_events, parse_dispatch_action, parse_tracker_event, TrackerFold,
};
use nostr::{Event, Kind};
use serde_json::{json, Value};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::config::DispatcherConfig;
use crate::emit::Emitter;
use crate::error::DispatchError;
use crate::projection::{InstanceLock, ProjectionStore, DISPATCHER_LOCK_FILE};
use crate::relay_http::{parse_submit_receipt, RelayHttp};
use crate::rules;

/// Run the dispatcher until the process is stopped.
pub async fn run(
    config: DispatcherConfig,
    relay: RelayHttp,
    instance_id: Uuid,
) -> Result<(), DispatchError> {
    // M7: single-instance guard — a second process (manual run + launchd, or
    // a respawn during a wedge) rename-clobbers the state files and loses
    // fired-ledger entries, so fail fast instead.
    let _lock = InstanceLock::acquire(&config.state_dir, DISPATCHER_LOCK_FILE)?;
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
    if config.remote_controls {
        let result = crate::remote_control::tick(config, relay, emitter, store).await;
        if let Err(error) = &result {
            let _ = emitter.emit_heartbeat(store, Some(error.to_string())).await;
        }
        return result;
    }
    // M2: replay anything that reached the outbox but not the fired ledger
    // before a crash. Duplicate-tolerant and idempotent; a permanently
    // rejected entry is retried a bounded number of times, then quarantined
    // — one poison event must not abort every tick (heartbeat included).
    replay_outbox(store, relay).await?;

    let since = store.since();

    // Inclusive refresh: query everything at/after the cursor. Event-id dedupe
    // in the store absorbs the boundary overlap. `now` is computed BEFORE
    // ingest so the cursor clamp (M1) bounds advancement at local time.
    let (fresh, reaction_channels) = fetch_new_events(config, relay, since).await?;
    let now = current_unix_seconds();
    // B5: record which monitored channel each reaction came from BEFORE
    // planning — real reactions carry no `h` tag to recover it from.
    store.record_reaction_channels(reaction_channels);
    let ingested = store.ingest(fresh, now);
    if ingested > 0 {
        debug!(count = ingested, "ingested new events");
    }

    // M3: fold ingested kind-40023 dispatch actions into the fired ledger —
    // covers external services' dispatches AND our own publications that
    // reached the relay but not the ledger before a crash. Authorization is
    // checked before the fold (same TrackerAuthorization the fold uses).
    observe_fired_actions(store, authorization, now);

    let fold = fold_store(store, authorization, now);

    // Track emission failures for the heartbeat health (no more hardcoded
    // Healthy when effects failed).
    let mut degraded: Vec<String> = Vec::new();

    // Capture plans first (reactions), then fold-driven plans.
    let captures = rules::plan_captures(config, store);
    for plan in &captures {
        if let Err(error) = emitter.emit_capture(store, plan).await {
            warn!(%error, source = %plan.source_event_id, "capture emission failed");
            degraded.push(format!("capture({})", short_id(&plan.source_event_id)));
        }
    }

    let actions = rules::plan_fold_actions(config, &fold, store, now);
    if !actions.is_empty() {
        if let Err(error) = emitter.emit_rule_actions(store, &actions).await {
            warn!(%error, count = actions.len(), "rule-action emission failed");
            degraded.push(format!("rule-actions({})", actions.len()));
        }
    }

    store.commit()?;

    // Heartbeat last so its cursor reflects everything committed this tick.
    let detail = if degraded.is_empty() {
        None
    } else {
        Some(format!(
            "{} emission failure(s): {}",
            degraded.len(),
            degraded.join("; ")
        ))
    };
    if let Err(error) = emitter.emit_heartbeat(store, detail).await {
        warn!(%error, "heartbeat emission failed");
    }

    Ok(())
}

/// Replay attempts after which a still-failing outbox entry is quarantined.
/// With the default 30s poll this is ~5 minutes of continuous rejection —
/// comfortably past transient relay blips, and quarantine loses nothing: the
/// request id stays unfired, so planning re-signs a fresh event afterwards.
const MAX_OUTBOX_RETRIES: u64 = 10;

/// Replay pending outbox publications (M2 persist-before-publish). Duplicate
/// receipts count as success: the current relay answers `accepted:true` for
/// events it already stores (the message may note the duplicate), and older
/// relays may answer `accepted:false, "duplicate: …"` — both are tolerated.
/// Any other failure bumps the entry's attempt counter; after
/// [`MAX_OUTBOX_RETRIES`] the entry is quarantined and the loop moves on, so
/// one permanently-rejected event cannot wedge the dispatcher.
async fn replay_outbox(
    store: &mut ProjectionStore,
    relay: &RelayHttp,
) -> Result<(), DispatchError> {
    let pending = store.pending_outbox().to_vec();
    for entry in pending {
        if let Err(error) = publish_outbox_entry(relay, &entry).await {
            let attempts = store.bump_outbox_attempts(&entry.request_id)?;
            if attempts >= MAX_OUTBOX_RETRIES {
                warn!(
                    request_id = %entry.request_id,
                    %error,
                    attempts,
                    "quarantining permanently-rejected outbox entry; the request re-plans with a fresh signature"
                );
                store.quarantine_outbox(&entry.request_id, &error.to_string())?;
            } else {
                warn!(
                    request_id = %entry.request_id,
                    %error,
                    attempts,
                    "outbox replay rejected; retrying on a later tick"
                );
            }
            continue;
        }
        store.mark_request_fired(entry.request_id);
        store.persist_ledger()?;
        store.complete_outbox(&entry.request_id)?;
        info!(
            request_id = %entry.request_id,
            "replayed pending outbox publication"
        );
    }
    Ok(())
}

/// Publish every event of one outbox entry in order. Transport failures and
/// non-duplicate rejections are returned for retry/quarantine by the caller.
async fn publish_outbox_entry(
    relay: &RelayHttp,
    entry: &crate::projection::OutboxEntry,
) -> Result<(), DispatchError> {
    for payload in &entry.events {
        let event: Event = serde_json::from_value(payload.clone())?;
        let raw = relay.submit_event(&event).await?;
        let receipt = parse_submit_receipt(&raw)?;
        if !receipt.accepted && !receipt.message.starts_with("duplicate") {
            return Err(DispatchError::Relay(format!(
                "outbox replay rejected {}: {}",
                event.id.to_hex(),
                receipt.message
            )));
        }
    }
    Ok(())
}

/// Fold parsed kind-40023 dispatch actions into the fired ledger (M3).
/// Strict parsing drops malformed events; the authorization gate drops
/// everyone else. Request ids are deterministic UUIDv5 over public stream
/// data and kind-40023 needs only channel write scope, so an unfiltered fold
/// would let ANY stream-channel member permanently suppress a capture, route,
/// or escalation by publishing a self-signed 40023 with the victim request
/// id. Only the service's own key and configured workspace roles pass — the
/// same [`buzz_sdk::tracker::TrackerAuthorization`] the fold uses, which the
/// config builds with the service pubkey merged in.
fn observe_fired_actions(
    store: &mut ProjectionStore,
    authorization: &buzz_sdk::tracker::TrackerAuthorization,
    now: u64,
) {
    let action_kind = Kind::Custom(buzz_sdk::kind::KIND_TRACKER_DISPATCH_ACTION as u16);
    let observed: Vec<Uuid> = store
        .events()
        .iter()
        .filter(|event| event.kind == action_kind)
        .filter(|event| authorization.role(&event.pubkey.to_hex()).is_some())
        .filter_map(|event| parse_dispatch_action(event, now).ok())
        .map(|parsed| parsed.action.request_id)
        .collect();
    if !observed.is_empty() {
        store.observe_fired_requests(observed);
    }
}

/// First 8 hex chars of an id for log lines.
fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

/// Fetch tracker-stream events, monitored-channel reactions, and prior dispatch
/// actions at/after `since`.
///
/// Both families paginate backward from the newest event with the bridge's
/// composite `(until, before_id)` cursor (B4): the relay clamps limitless
/// filters to the newest 1000 rows, so a single unpaginated refresh against a
/// >1000-event stream silently gaps everything older.
///
/// Returns the events plus a reaction-id → home-channel map (B5): real kind-7
/// reactions carry no `h` tag — the relay derives the channel from the
/// reaction's target and indexes by `#h` without materializing the tag — so
/// the per-channel query that fetched each reaction is its only channel
/// source.
async fn fetch_new_events(
    config: &DispatcherConfig,
    relay: &RelayHttp,
    since: u64,
) -> Result<(Vec<Event>, std::collections::HashMap<String, Uuid>), DispatchError> {
    let mut events = Vec::new();

    // Tracker stream: roots, revisions, comments, dispatch actions.
    events.extend(query_paginated_events(relay, tracker_refresh_filter(config, since)).await?);

    // Monitored-channel pin reactions for capture. One query PER channel (B5)
    // so every returned reaction can be attributed to its home channel.
    let mut reaction_channels = std::collections::HashMap::new();
    for channel_id in &config.monitored_channels {
        let fetched =
            query_paginated_events(relay, reaction_refresh_filter(channel_id, since)).await?;
        for event in &fetched {
            reaction_channels.insert(event.id.to_hex(), *channel_id);
        }
        events.extend(fetched);
    }

    Ok((events, reaction_channels))
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
/// B1). One filter per channel (B5): the channel attributed to each returned
/// reaction comes from the filter, since real reactions have no `h` tag.
fn reaction_refresh_filter(channel: &Uuid, since: u64) -> Value {
    json!({ "kinds": [7], "#h": [channel.to_string()], "since": since })
}

/// Page size for backward refresh pagination. Below the relay's 1000-row page
/// clamp; matches the buzz-cli `QUERY_PAGE_SIZE`.
const REFRESH_PAGE_SIZE: usize = 500;
/// Hard cap on pages per refresh so a pathological stream cannot spin the
/// tick forever (500 * 200 = 100k events per family per tick).
const REFRESH_MAX_PAGES: usize = 200;

/// Query one filter backward across bounded pages until the result is
/// exhausted, newest page first. The base filter carries the inclusive
/// `since`; every non-empty page advances the composite `(until, before_id)`
/// cursor, exactly the shape `buzz-cli query_pages` uses.
///
/// Termination is an EMPTY page, not a short one: the bridge documents that
/// `rows < limit` proves nothing under its post-query filters. This crate's
/// filters are fully SQL-pushable today, so short pages also terminate in
/// practice (at the cost of one trailing empty query), but short-page
/// termination would be a latent gap trap if a future filter gained
/// post-processing.
///
/// An incomplete refresh is an error. Its newest event cannot advance the
/// inclusive high-water cursor: doing so would permanently skip the older pages.
pub(crate) async fn query_paginated_events(
    relay: &RelayHttp,
    mut filter: Value,
) -> Result<Vec<Event>, DispatchError> {
    let mut events = Vec::new();
    let mut previous_cursor = None;
    for _ in 0..REFRESH_MAX_PAGES {
        filter["limit"] = json!(REFRESH_PAGE_SIZE);
        let page = query_page(relay, &filter).await?;
        if page.is_empty() {
            return Ok(events);
        }
        advance_refresh_cursor(&mut filter, &page)?;
        let cursor = (filter["until"].clone(), filter["before_id"].clone());
        if previous_cursor.as_ref() == Some(&cursor) {
            return Err(DispatchError::Relay(
                "history pagination made no progress; cursor was not advanced".into(),
            ));
        }
        previous_cursor = Some(cursor);
        events.extend(page);
    }
    Err(DispatchError::Relay(
        "history page budget exhausted; incomplete history cannot advance the refresh cursor"
            .into(),
    ))
}

/// One `POST /query` page; unparseable events are skipped with a warning.
async fn query_page(relay: &RelayHttp, filter: &Value) -> Result<Vec<Event>, DispatchError> {
    let values = relay.query(std::slice::from_ref(filter)).await?;
    let mut events = Vec::with_capacity(values.len());
    for value in values {
        match serde_json::from_value::<Event>(value) {
            Ok(event) => events.push(event),
            Err(error) => {
                return Err(DispatchError::Relay(format!(
                    "incomplete history: a query event could not be decoded: {error}"
                )))
            }
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
            remote_controls: false,
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
                signer_allowlist: vec!["c".repeat(64)],
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

        let reactions = reaction_refresh_filter(&config.monitored_channels[0], 42);
        assert!(reactions.is_object(), "reaction filter must be an object");
        assert_eq!(reactions["kinds"], json!([7]));
        assert_eq!(reactions["since"], json!(42));
        // B5: one filter per channel, so the query carries the channel
        // context real reactions have no `h` tag for.
        assert_eq!(
            reactions["#h"],
            json!([config.monitored_channels[0].to_string()])
        );

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

    /// M3 wiring: ingested kind-40023 dispatch actions fold into the fired
    /// ledger, suppressing re-planning of requests another writer (or our own
    /// pre-crash publication) already fired.
    #[test]
    fn observe_fired_actions_folds_valid_dispatch_actions() {
        use crate::projection::tests::TEST_NOW;
        use buzz_sdk::tracker::{
            build_dispatch_action, DispatchAction, DispatchActionKind, TRACKER_VERSION,
        };

        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = ProjectionStore::open(dir.path()).expect("open");

        let stream = Uuid::new_v4();
        let request_id = Uuid::new_v4();
        let action = DispatchAction {
            v: TRACKER_VERSION,
            issue_id: Uuid::new_v4(),
            root_event: "a".repeat(64),
            request_id,
            action: DispatchActionKind::Assign,
            threshold_id: None,
            observed_tip: None,
            result: "test".to_string(),
        };
        let keys = Keys::generate();
        let event = build_dispatch_action(stream, &action)
            .expect("built")
            .sign_with_keys(&keys)
            .expect("signed");

        let authorization = buzz_sdk::tracker::TrackerAuthorization::new([(
            keys.public_key().to_hex(),
            buzz_sdk::tracker::TrackerRole::Member,
        )]);

        assert!(!store.request_fired(&request_id));
        store.ingest(vec![event], TEST_NOW);
        observe_fired_actions(&mut store, &authorization, TEST_NOW);
        assert!(
            store.request_fired(&request_id),
            "valid 40023 from an authorized writer must land in the fired ledger"
        );

        // Garbage of the same kind is dropped silently, not an error.
        let junk = EventBuilder::new(
            Kind::Custom(buzz_sdk::kind::KIND_TRACKER_DISPATCH_ACTION as u16),
            "not-a-dispatch-action",
        )
        .sign_with_keys(&keys)
        .expect("signed");
        store.ingest(vec![junk], TEST_NOW);
        observe_fired_actions(&mut store, &authorization, TEST_NOW);
    }

    /// M3 authorization gate: request ids are deterministic UUIDv5 over
    /// public stream data and kind-40023 needs only channel write scope, so
    /// a 40023 signed by an UNAUTHORIZED stream member must NOT reach the
    /// fired ledger — otherwise any member could permanently suppress a
    /// capture, route, or escalation by spoofing the victim request id.
    #[test]
    fn observe_fired_actions_ignores_unauthorized_authors() {
        use crate::projection::tests::TEST_NOW;
        use buzz_sdk::tracker::{
            build_dispatch_action, DispatchAction, DispatchActionKind, TRACKER_VERSION,
        };

        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = ProjectionStore::open(dir.path()).expect("open");

        let stream = Uuid::new_v4();
        let request_id = Uuid::new_v4();
        let action = DispatchAction {
            v: TRACKER_VERSION,
            issue_id: Uuid::new_v4(),
            root_event: "a".repeat(64),
            request_id,
            action: DispatchActionKind::Assign,
            threshold_id: None,
            observed_tip: None,
            result: "spoof".to_string(),
        };
        let attacker = Keys::generate();
        let spoofed = build_dispatch_action(stream, &action)
            .expect("built")
            .sign_with_keys(&attacker)
            .expect("signed");

        // Authorization that does NOT include the attacker's pubkey.
        let authorization = buzz_sdk::tracker::TrackerAuthorization::new([(
            "0".repeat(64),
            buzz_sdk::tracker::TrackerRole::Maintainer,
        )]);

        store.ingest(vec![spoofed], TEST_NOW);
        observe_fired_actions(&mut store, &authorization, TEST_NOW);
        assert!(
            !store.request_fired(&request_id),
            "unauthorized 40023 must not poison the fired ledger"
        );
    }
}
