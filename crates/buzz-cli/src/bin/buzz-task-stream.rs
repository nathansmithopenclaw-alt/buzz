//! Authenticated relay stream for Gateway-supervised task-envelope ingestion.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use buzz_sdk::task_envelope::{TaskEnvelopeEvent, TASK_DISCOVERY_HASHTAG};
use buzz_ws_client::{NostrWsConnection, RelayMessage};
use clap::Parser;
use nostr::Keys;
use serde::Serialize;
use serde_json::json;

const CHANNEL_DISCOVERY_SUBSCRIPTION_ID: &str = "buzz-task-channel-discovery-v1";
const TASK_SUBSCRIPTION_PREFIX: &str = "buzz-task-kernel-bridge-v1";
const DEFAULT_RELAY_URL: &str = "wss://tituss-mac-mini.tail561ede.ts.net:8443";
const BACKFILL_EOSE_DEADLINE: Duration = Duration::from_secs(30);

#[derive(Debug, Parser)]
#[command(about = "Stream SDK-validated Buzz task envelopes as NDJSON")]
struct Args {
    /// Generate a dedicated bridge keypair as JSON for a provisioning pipe.
    #[arg(long)]
    generate_key: bool,

    /// Relay WebSocket URL. The host must map to a relay community.
    #[arg(long, default_value = DEFAULT_RELAY_URL)]
    relay: String,

    /// Last durably processed event timestamp.
    #[arg(long, default_value_t = 0)]
    cursor_created_at: u64,

    /// Last durably processed event ID, skipped exactly once during replay.
    #[arg(long, default_value = "")]
    cursor_event_id: String,
}

#[derive(Serialize)]
struct GeneratedKeypair {
    private_key: String,
    public_key: String,
}

fn new_keypair() -> GeneratedKeypair {
    let keys = Keys::generate();
    GeneratedKeypair {
        private_key: keys.secret_key().display_secret().to_string(),
        public_key: keys.public_key().to_hex(),
    }
}

fn generate_keypair() -> Result<(), String> {
    serde_json::to_writer(io::stdout().lock(), &new_keypair())
        .map_err(|error| format!("failed to serialize generated keypair: {error}"))
}

fn read_keys() -> Result<Keys, String> {
    let mut secret = String::new();
    io::stdin()
        .read_to_string(&mut secret)
        .map_err(|error| format!("failed to read bridge key from stdin: {error}"))?;
    let keys = Keys::parse(secret.trim()).map_err(|_| "invalid bridge private key".to_string())?;
    secret.clear();
    Ok(keys)
}

#[derive(Serialize)]
struct StreamTaskEvent<'a> {
    #[serde(flatten)]
    event: &'a TaskEnvelopeEvent,
    bridge_pubkey: &'a str,
}

#[derive(Debug, Serialize)]
struct StreamRejectionRecord {
    record_type: &'static str,
    event_id: String,
    author_pubkey: String,
    channel_id: String,
    created_at: u64,
    bridge_pubkey: String,
    error: String,
}

#[derive(Debug)]
enum BufferedRecord {
    Task(TaskEnvelopeEvent),
    Rejection(StreamRejectionRecord),
}

impl BufferedRecord {
    fn sort_key(&self) -> (u64, &str) {
        match self {
            Self::Task(event) => (event.created_at, event.event_id.as_str()),
            Self::Rejection(record) => (record.created_at, record.event_id.as_str()),
        }
    }
}

fn emit_event(event: &TaskEnvelopeEvent, bridge_pubkey: &str) -> Result<(), String> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(
        &mut stdout,
        &StreamTaskEvent {
            event,
            bridge_pubkey,
        },
    )
    .map_err(|error| format!("failed to serialize task event: {error}"))?;
    stdout
        .write_all(b"\n")
        .and_then(|_| stdout.flush())
        .map_err(|error| format!("failed to emit task event: {error}"))
}

fn emit_record(record: &BufferedRecord, bridge_pubkey: &str) -> Result<(), String> {
    match record {
        BufferedRecord::Task(event) => emit_event(event, bridge_pubkey),
        BufferedRecord::Rejection(record) => {
            let mut stdout = io::stdout().lock();
            serde_json::to_writer(&mut stdout, record)
                .map_err(|error| format!("failed to serialize rejection record: {error}"))?;
            stdout
                .write_all(b"\n")
                .and_then(|_| stdout.flush())
                .map_err(|error| format!("failed to emit rejection record: {error}"))
        }
    }
}

fn rejection_record(
    event: &nostr::Event,
    channel_id: &str,
    bridge_pubkey: &str,
    error: &str,
) -> StreamRejectionRecord {
    StreamRejectionRecord {
        record_type: "rejection",
        event_id: event.id.to_hex(),
        author_pubkey: event.pubkey.to_hex(),
        channel_id: channel_id.to_string(),
        created_at: event.created_at.as_secs(),
        bridge_pubkey: bridge_pubkey.to_string(),
        error: error.chars().take(512).collect(),
    }
}

fn membership_channel_id(event: &nostr::Event, bridge_pubkey: &str) -> Option<String> {
    if event.kind.as_u16() != 39002 {
        return None;
    }
    let is_member = event.tags.iter().any(|tag| {
        let values = tag.as_slice();
        values.first().map(String::as_str) == Some("p")
            && values.get(1).map(String::as_str) == Some(bridge_pubkey)
    });
    if !is_member {
        return None;
    }
    event.tags.iter().find_map(|tag| {
        let values = tag.as_slice();
        (values.first().map(String::as_str) == Some("d"))
            .then(|| values.get(1))
            .flatten()
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .map(|channel_id| channel_id.to_string())
    })
}

async fn start_channel_discovery(
    connection: &mut NostrWsConnection,
    bridge_pubkey: &str,
) -> Result<(), String> {
    connection
        .send_raw(&json!([
            "REQ",
            CHANNEL_DISCOVERY_SUBSCRIPTION_ID,
            {
                "kinds": [39002],
                "#p": [bridge_pubkey],
            }
        ]))
        .await
        .map_err(|error| format!("relay channel discovery failed: {error}"))
}

fn task_subscription(channel_id: &str, since: u64) -> (String, serde_json::Value) {
    let subscription_id = format!("{TASK_SUBSCRIPTION_PREFIX}-{channel_id}");
    let request = json!([
        "REQ",
        subscription_id,
        {
            "kinds": [9],
            "#h": [channel_id],
            "#t": [TASK_DISCOVERY_HASHTAG],
            "since": since,
        }
    ]);
    (subscription_id, request)
}

struct TaskSubscriptionState {
    backfill: Vec<BufferedRecord>,
    backfill_complete: bool,
    backfill_deadline: Instant,
}

impl TaskSubscriptionState {
    fn new(now: Instant) -> Self {
        Self {
            backfill: Vec::new(),
            backfill_complete: false,
            backfill_deadline: now + BACKFILL_EOSE_DEADLINE,
        }
    }
}

fn complete_channel_backfill(state: &mut TaskSubscriptionState) -> Vec<BufferedRecord> {
    state.backfill_complete = true;
    state
        .backfill
        .sort_by(|left, right| left.sort_key().cmp(&right.sort_key()));
    std::mem::take(&mut state.backfill)
}

fn event_is_not_cursor(event: &TaskEnvelopeEvent, args: &Args) -> bool {
    args.cursor_event_id.is_empty()
        || event.created_at != args.cursor_created_at
        || event.event_id != args.cursor_event_id
}

async fn ensure_task_subscription(
    connection: &mut NostrWsConnection,
    subscriptions: &mut BTreeMap<String, TaskSubscriptionState>,
    channel_id: &str,
    since: u64,
) -> Result<(), String> {
    let (subscription_id, request) = task_subscription(channel_id, since);
    if subscriptions.contains_key(&subscription_id) {
        return Ok(());
    }
    connection
        .send_raw(&request)
        .await
        .map_err(|error| format!("relay task subscription failed: {error}"))?;
    subscriptions.insert(subscription_id, TaskSubscriptionState::new(Instant::now()));
    Ok(())
}

fn expired_backfill_subscriptions(
    subscriptions: &BTreeMap<String, TaskSubscriptionState>,
    now: Instant,
) -> Vec<String> {
    subscriptions
        .iter()
        .filter(|(_, state)| !state.backfill_complete && now >= state.backfill_deadline)
        .map(|(subscription_id, _)| subscription_id.clone())
        .collect()
}

fn complete_expired_backfills(
    subscriptions: &mut BTreeMap<String, TaskSubscriptionState>,
    now: Instant,
) -> Vec<(String, Vec<BufferedRecord>)> {
    expired_backfill_subscriptions(subscriptions, now)
        .into_iter()
        .filter_map(|subscription_id| {
            subscriptions
                .get_mut(&subscription_id)
                .map(complete_channel_backfill)
                .map(|events| (subscription_id, events))
        })
        .collect()
}

fn next_event_timeout(
    subscriptions: &BTreeMap<String, TaskSubscriptionState>,
    now: Instant,
) -> Duration {
    subscriptions
        .values()
        .filter(|state| !state.backfill_complete)
        .map(|state| state.backfill_deadline.saturating_duration_since(now))
        .min()
        .unwrap_or(Duration::from_secs(90))
        .min(Duration::from_secs(90))
}

async fn stream_once(args: &Args, keys: &Keys) -> Result<(), String> {
    let mut connection = NostrWsConnection::connect_authenticated(&args.relay, keys, None)
        .await
        .map_err(|error| format!("relay connect/auth failed: {error}"))?;
    let bridge_pubkey = keys.public_key().to_hex();
    start_channel_discovery(&mut connection, &bridge_pubkey).await?;
    let since = args.cursor_created_at.saturating_sub(1);
    let mut discovery_complete = false;
    let mut subscriptions = BTreeMap::new();
    loop {
        for (subscription_id, events) in
            complete_expired_backfills(&mut subscriptions, Instant::now())
        {
            eprintln!("relay task backfill EOSE deadline elapsed for {subscription_id}; emitting buffered events");
            for record in events {
                emit_record(&record, &bridge_pubkey)?;
            }
        }
        let wait = next_event_timeout(&subscriptions, Instant::now());
        match connection.next_event(wait).await {
            Ok(RelayMessage::Event {
                subscription_id,
                event,
            }) if subscription_id == CHANNEL_DISCOVERY_SUBSCRIPTION_ID => {
                if let Some(channel_id) = membership_channel_id(&event, &bridge_pubkey) {
                    ensure_task_subscription(
                        &mut connection,
                        &mut subscriptions,
                        &channel_id,
                        since,
                    )
                    .await?;
                }
            }
            Ok(RelayMessage::Eose { subscription_id })
                if subscription_id == CHANNEL_DISCOVERY_SUBSCRIPTION_ID =>
            {
                discovery_complete = true;
                if subscriptions.is_empty() {
                    return Err("bridge identity is not a member of any channels".to_string());
                }
            }
            Ok(RelayMessage::Closed {
                subscription_id,
                message,
            }) if subscription_id == CHANNEL_DISCOVERY_SUBSCRIPTION_ID => {
                return Err(format!("relay closed channel discovery: {message}"));
            }
            Ok(RelayMessage::Event {
                subscription_id: event_subscription,
                event,
            }) if event_subscription.starts_with(TASK_SUBSCRIPTION_PREFIX) => {
                let Some(state) = subscriptions.get_mut(&event_subscription) else {
                    continue;
                };
                match TaskEnvelopeEvent::from_event(&event) {
                    Ok(task_event) => {
                        if !event_is_not_cursor(&task_event, args) {
                            continue;
                        }
                        if state.backfill_complete {
                            emit_event(&task_event, &bridge_pubkey)?;
                        } else {
                            state.backfill.push(BufferedRecord::Task(task_event));
                        }
                    }
                    Err(error) => {
                        let Some(channel_id) = event_subscription
                            .strip_prefix(TASK_SUBSCRIPTION_PREFIX)
                            .and_then(|suffix| suffix.strip_prefix('-'))
                        else {
                            continue;
                        };
                        let rejection = rejection_record(
                            &event,
                            channel_id,
                            &bridge_pubkey,
                            &error.to_string(),
                        );
                        if state.backfill_complete {
                            emit_record(&BufferedRecord::Rejection(rejection), &bridge_pubkey)?;
                        } else {
                            state.backfill.push(BufferedRecord::Rejection(rejection));
                        }
                    }
                }
            }
            Ok(RelayMessage::Eose {
                subscription_id: event_subscription,
            }) if event_subscription.starts_with(TASK_SUBSCRIPTION_PREFIX) => {
                if let Some(state) = subscriptions.get_mut(&event_subscription) {
                    for record in complete_channel_backfill(state) {
                        emit_record(&record, &bridge_pubkey)?;
                    }
                }
            }
            Ok(RelayMessage::Closed {
                subscription_id,
                message,
            }) if subscription_id.starts_with(TASK_SUBSCRIPTION_PREFIX) => {
                return Err(format!("relay closed task subscription: {message}"));
            }
            Ok(RelayMessage::Notice { message }) => eprintln!("relay notice: {message}"),
            Ok(_) => {}
            Err(error) => {
                if !expired_backfill_subscriptions(&subscriptions, Instant::now()).is_empty() {
                    continue;
                }
                let phase = if discovery_complete {
                    "relay stream"
                } else {
                    "relay channel discovery"
                };
                return Err(format!("{phase} failed: {error}"));
            }
        }
    }
}

fn stream_error_exit_code(error: &str) -> i32 {
    if error.starts_with("relay connect/auth failed:") {
        3
    } else {
        2
    }
}

#[tokio::main]
async fn main() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let args = Args::parse();
    if args.generate_key {
        if let Err(error) = generate_keypair() {
            eprintln!("{error}");
            std::process::exit(4);
        }
        return;
    }
    let keys = match read_keys() {
        Ok(keys) => keys,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(3);
        }
    };

    if let Err(error) = stream_once(&args, &keys).await {
        eprintln!("{error}");
        std::process::exit(stream_error_exit_code(&error));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_sdk::task_envelope::{TaskEnvelope, TaskStatus, TaskVerification};
    use nostr::{EventBuilder, Kind, Tag};
    use uuid::Uuid;

    fn event(created_at: u64, event_id: &str) -> TaskEnvelopeEvent {
        TaskEnvelopeEvent {
            event_id: event_id.to_string(),
            author_pubkey: "11".repeat(32),
            channel_id: Uuid::nil(),
            created_at,
            envelope: TaskEnvelope {
                v: 1,
                task_id: Uuid::nil(),
                status: TaskStatus::Assigned,
                assignee: "22".repeat(32),
                assigner: "11".repeat(32),
                title: "test".into(),
                deliverables: Vec::new(),
                verification: TaskVerification {
                    method: "test".into(),
                    evidence_ref: None,
                },
                blocked_on: None,
                notes: None,
            },
        }
    }

    fn membership_event(channel_id: Uuid, members: &[&str]) -> nostr::Event {
        let mut tags = vec![Tag::parse(["d", channel_id.to_string().as_str()]).unwrap()];
        for member in members {
            tags.push(Tag::parse(["p", *member]).unwrap());
        }
        EventBuilder::new(Kind::Custom(39002), "")
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn event_fixture_preserves_cursor_tuple_fields() {
        let event = event(100, "b");
        assert_eq!((event.created_at, event.event_id.as_str()), (100, "b"));
    }

    #[test]
    fn malformed_envelope_rejection_record_is_metadata_only() {
        let channel_id = Uuid::new_v4().to_string();
        let raw_event = EventBuilder::new(Kind::Custom(9), "MIRRORED CONTENT MUST NOT LEAK")
            .tags([
                Tag::parse(["h", channel_id.as_str()]).unwrap(),
                Tag::parse(["t", TASK_DISCOVERY_HASHTAG]).unwrap(),
                Tag::parse(["task", "malformed"]).unwrap(),
            ])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let record = rejection_record(
            &raw_event,
            &channel_id,
            &"bb".repeat(32),
            "task envelope event requires exactly one well-formed task tag",
        );
        let value = serde_json::to_value(&record).unwrap();

        assert_eq!(value["record_type"], "rejection");
        assert_eq!(value["event_id"], raw_event.id.to_hex());
        assert_eq!(value["channel_id"], channel_id);
        assert_eq!(value["created_at"], raw_event.created_at.as_secs());
        assert!(value.get("content").is_none());
        assert!(!value.to_string().contains("MIRRORED CONTENT MUST NOT LEAK"));
    }

    #[test]
    fn cursor_event_id_skips_only_the_exact_processed_event() {
        let args = Args::parse_from([
            "buzz-task-stream",
            "--cursor-created-at",
            "100",
            "--cursor-event-id",
            "b",
        ]);

        assert!(event_is_not_cursor(&event(99, "z"), &args));
        assert!(event_is_not_cursor(&event(100, "a"), &args));
        assert!(!event_is_not_cursor(&event(100, "b"), &args));
        assert!(event_is_not_cursor(&event(100, "c"), &args));
        assert!(event_is_not_cursor(&event(101, "a"), &args));

        let timestamp_only = Args::parse_from(["buzz-task-stream", "--cursor-created-at", "100"]);
        assert!(event_is_not_cursor(&event(100, "a"), &timestamp_only));
    }

    #[test]
    fn generated_keypair_round_trips_to_its_public_key() {
        let generated = new_keypair();
        let keys = Keys::parse(&generated.private_key).expect("generated private key");
        assert_eq!(keys.public_key().to_hex(), generated.public_key);
    }

    #[test]
    fn task_subscription_is_channel_scoped_for_live_fanout() {
        let channel_id = uuid::Uuid::new_v4().to_string();
        let (subscription_id, request) = task_subscription(&channel_id, 99);
        assert_eq!(
            subscription_id,
            format!("{TASK_SUBSCRIPTION_PREFIX}-{channel_id}")
        );
        assert_eq!(request[2]["#h"], json!([channel_id]));
        assert_eq!(request[2]["#t"], json!([TASK_DISCOVERY_HASHTAG]));
        assert_eq!(request[2]["since"], json!(99));
    }

    #[test]
    fn channel_backfills_complete_independently() {
        let now = Instant::now();
        let mut first = TaskSubscriptionState::new(now);
        let mut second = TaskSubscriptionState::new(now);
        first.backfill.push(BufferedRecord::Task(event(101, "b")));
        first.backfill.push(BufferedRecord::Task(event(100, "a")));
        first
            .backfill
            .push(BufferedRecord::Rejection(StreamRejectionRecord {
                record_type: "rejection",
                event_id: "0".repeat(64),
                author_pubkey: "aa".repeat(32),
                channel_id: Uuid::new_v4().to_string(),
                created_at: 100,
                bridge_pubkey: "bb".repeat(32),
                error: "malformed envelope".to_string(),
            }));
        second.backfill.push(BufferedRecord::Task(event(99, "z")));

        let emitted = complete_channel_backfill(&mut first);

        assert_eq!(
            emitted
                .iter()
                .map(|record| record.sort_key().1)
                .collect::<Vec<_>>(),
            vec![
                "0000000000000000000000000000000000000000000000000000000000000000",
                "a",
                "b"
            ]
        );
        assert!(first.backfill_complete);
        assert!(!second.backfill_complete);
        assert_eq!(second.backfill.len(), 1);
    }

    #[test]
    fn expired_channel_backfill_is_released_without_waiting_for_other_channels() {
        let now = Instant::now();
        let mut subscriptions = BTreeMap::new();
        subscriptions.insert(
            "expired".to_string(),
            TaskSubscriptionState {
                backfill: vec![BufferedRecord::Task(event(100, "a"))],
                backfill_complete: false,
                backfill_deadline: now,
            },
        );
        subscriptions.insert("waiting".to_string(), TaskSubscriptionState::new(now));

        assert_eq!(
            expired_backfill_subscriptions(&subscriptions, now),
            vec!["expired"]
        );
        assert_eq!(next_event_timeout(&subscriptions, now), Duration::ZERO);
    }

    #[test]
    fn chatty_other_channel_does_not_extend_a_missing_eose_deadline() {
        let now = Instant::now();
        let mut subscriptions = BTreeMap::new();
        let mut stalled = TaskSubscriptionState::new(now);
        stalled
            .backfill
            .push(BufferedRecord::Task(event(100, "stalled")));
        subscriptions.insert("stalled".to_string(), stalled);

        let mut chatty = TaskSubscriptionState::new(now);
        chatty.backfill_complete = true;
        subscriptions.insert("chatty".to_string(), chatty);
        subscriptions.insert(
            "later".to_string(),
            TaskSubscriptionState::new(now + Duration::from_secs(15)),
        );

        for elapsed in [1, 5, 10, 20, 29] {
            assert!(complete_expired_backfills(
                &mut subscriptions,
                now + Duration::from_secs(elapsed),
            )
            .is_empty());
            assert!(subscriptions["chatty"].backfill_complete);
        }

        let released = complete_expired_backfills(&mut subscriptions, now + BACKFILL_EOSE_DEADLINE);
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].0, "stalled");
        assert!(matches!(
            &released[0].1[0],
            BufferedRecord::Task(event) if event.event_id == "stalled"
        ));
        assert!(subscriptions["stalled"].backfill_complete);
        assert!(subscriptions["chatty"].backfill_complete);
        assert!(!subscriptions["later"].backfill_complete);
    }

    #[test]
    fn auth_failures_request_secret_invalidation_exit_code() {
        assert_eq!(
            stream_error_exit_code("relay connect/auth failed: relay rejected authentication"),
            3
        );
        assert_eq!(stream_error_exit_code("relay stream failed: timeout"), 2);
    }

    #[test]
    fn canonical_relay_default_includes_mapped_host_and_port() {
        let args = Args::parse_from(["buzz-task-stream"]);
        assert_eq!(args.relay, DEFAULT_RELAY_URL);
    }

    #[test]
    fn membership_snapshots_add_only_channels_containing_the_bridge() {
        let channel_id = Uuid::new_v4();
        let bridge_pubkey = "11".repeat(32);
        let included = membership_event(channel_id, &[bridge_pubkey.as_str(), &"22".repeat(32)]);
        let removed = membership_event(channel_id, &[&"22".repeat(32)]);

        assert_eq!(
            membership_channel_id(&included, &bridge_pubkey),
            Some(channel_id.to_string())
        );
        assert_eq!(membership_channel_id(&removed, &bridge_pubkey), None);
    }
}
