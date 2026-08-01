//! Authenticated relay stream for Gateway-supervised task-envelope ingestion.

use std::collections::BTreeSet;
use std::io::{self, Read, Write};
use std::time::Duration;

use buzz_sdk::task_envelope::{TaskEnvelopeEvent, TASK_DISCOVERY_HASHTAG};
use buzz_ws_client::{NostrWsConnection, RelayMessage};
use clap::Parser;
use nostr::Keys;
use serde::Serialize;
use serde_json::json;

const CHANNEL_DISCOVERY_SUBSCRIPTION_ID: &str = "buzz-task-channel-discovery-v1";
const TASK_SUBSCRIPTION_PREFIX: &str = "buzz-task-kernel-bridge-v1";

#[derive(Debug, Parser)]
#[command(about = "Stream SDK-validated Buzz task envelopes as NDJSON")]
struct Args {
    /// Generate a dedicated bridge keypair as JSON for a provisioning pipe.
    #[arg(long)]
    generate_key: bool,

    /// Local relay WebSocket URL.
    #[arg(long, default_value = "ws://localhost:3000")]
    relay: String,

    /// Last durably processed event timestamp.
    #[arg(long, default_value_t = 0)]
    cursor_created_at: u64,

    /// Last durably processed event ID, used to order same-second events.
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

async fn discover_member_channels(
    connection: &mut NostrWsConnection,
    bridge_pubkey: &str,
) -> Result<Vec<String>, String> {
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
        .map_err(|error| format!("relay channel discovery failed: {error}"))?;

    let mut channel_ids = BTreeSet::new();
    loop {
        match connection.next_event(Duration::from_secs(30)).await {
            Ok(RelayMessage::Event {
                subscription_id,
                event,
            }) if subscription_id == CHANNEL_DISCOVERY_SUBSCRIPTION_ID => {
                if let Some(channel_id) = membership_channel_id(&event, bridge_pubkey) {
                    channel_ids.insert(channel_id);
                }
            }
            Ok(RelayMessage::Eose { subscription_id })
                if subscription_id == CHANNEL_DISCOVERY_SUBSCRIPTION_ID =>
            {
                break;
            }
            Ok(RelayMessage::Closed {
                subscription_id,
                message,
            }) if subscription_id == CHANNEL_DISCOVERY_SUBSCRIPTION_ID => {
                return Err(format!("relay closed channel discovery: {message}"));
            }
            Ok(RelayMessage::Notice { message }) => eprintln!("relay notice: {message}"),
            Ok(_) => {}
            Err(error) => return Err(format!("relay channel discovery failed: {error}")),
        }
    }
    connection
        .send_raw(&json!(["CLOSE", CHANNEL_DISCOVERY_SUBSCRIPTION_ID]))
        .await
        .map_err(|error| format!("relay channel discovery close failed: {error}"))?;
    Ok(channel_ids.into_iter().collect())
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

async fn stream_once(args: &Args, keys: &Keys) -> Result<(), String> {
    let mut connection = NostrWsConnection::connect_authenticated(&args.relay, keys, None)
        .await
        .map_err(|error| format!("relay connect/auth failed: {error}"))?;
    let bridge_pubkey = keys.public_key().to_hex();
    let channel_ids = discover_member_channels(&mut connection, &bridge_pubkey).await?;
    if channel_ids.is_empty() {
        return Err("bridge identity is not a member of any channels".to_string());
    }
    let since = args.cursor_created_at.saturating_sub(1);
    let mut pending_eose = BTreeSet::new();
    for channel_id in channel_ids {
        let (subscription_id, request) = task_subscription(&channel_id, since);
        pending_eose.insert(subscription_id);
        connection
            .send_raw(&request)
            .await
            .map_err(|error| format!("relay task subscription failed: {error}"))?;
    }

    let mut backfill = Vec::new();
    loop {
        match connection.next_event(Duration::from_secs(90)).await {
            Ok(RelayMessage::Event {
                subscription_id: event_subscription,
                event,
            }) if event_subscription.starts_with(TASK_SUBSCRIPTION_PREFIX) => {
                match TaskEnvelopeEvent::from_event(&event) {
                    Ok(task_event) => {
                        if pending_eose.is_empty() {
                            emit_event(&task_event, &bridge_pubkey)?;
                        } else {
                            backfill.push(task_event);
                        }
                    }
                    Err(error) => eprintln!("invalid task envelope skipped: {error}"),
                }
            }
            Ok(RelayMessage::Eose {
                subscription_id: event_subscription,
            }) if event_subscription.starts_with(TASK_SUBSCRIPTION_PREFIX) => {
                pending_eose.remove(&event_subscription);
                if pending_eose.is_empty() {
                    backfill.sort_by(|left, right| {
                        (left.created_at, left.event_id.as_str())
                            .cmp(&(right.created_at, right.event_id.as_str()))
                    });
                    for task_event in backfill.drain(..) {
                        emit_event(&task_event, &bridge_pubkey)?;
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
            Err(error) => return Err(format!("relay stream failed: {error}")),
        }
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
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_sdk::task_envelope::{TaskEnvelope, TaskStatus, TaskVerification};
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

    #[test]
    fn event_fixture_preserves_cursor_tuple_fields() {
        let event = event(100, "b");
        assert_eq!((event.created_at, event.event_id.as_str()), (100, "b"));
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
}
