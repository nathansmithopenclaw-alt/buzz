//! Authenticated relay stream for Gateway-supervised task-envelope ingestion.

use std::io::{self, Read, Write};
use std::time::Duration;

use buzz_sdk::task_envelope::{TaskEnvelopeEvent, TASK_DISCOVERY_HASHTAG};
use buzz_ws_client::{NostrWsConnection, RelayMessage};
use clap::Parser;
use nostr::Keys;
use serde::Serialize;
use serde_json::json;

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

async fn stream_once(args: &Args, keys: &Keys) -> Result<(), String> {
    let mut connection = NostrWsConnection::connect_authenticated(&args.relay, keys, None)
        .await
        .map_err(|error| format!("relay connect/auth failed: {error}"))?;
    let subscription_id = "buzz-task-kernel-bridge-v1";
    connection
        .send_raw(&json!([
            "REQ",
            subscription_id,
            {
                "kinds": [9],
                "#t": [TASK_DISCOVERY_HASHTAG],
                // Replay a one-second overlap. Events published live in the same
                // second are not guaranteed to arrive in event-id order; the
                // Gateway relies on kernel idempotency to absorb the overlap.
                "since": args.cursor_created_at.saturating_sub(1),
            }
        ]))
        .await
        .map_err(|error| format!("relay subscription failed: {error}"))?;

    let mut backfill = Vec::new();
    let mut backfill_complete = false;
    let bridge_pubkey = keys.public_key().to_hex();
    loop {
        match connection.next_event(Duration::from_secs(90)).await {
            Ok(RelayMessage::Event {
                subscription_id: event_subscription,
                event,
            }) if event_subscription == subscription_id => {
                match TaskEnvelopeEvent::from_event(&event) {
                    Ok(task_event) => {
                        if backfill_complete {
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
            }) if event_subscription == subscription_id => {
                backfill.sort_by(|left, right| {
                    (left.created_at, left.event_id.as_str())
                        .cmp(&(right.created_at, right.event_id.as_str()))
                });
                for task_event in backfill.drain(..) {
                    emit_event(&task_event, &bridge_pubkey)?;
                }
                backfill_complete = true;
            }
            Ok(RelayMessage::Closed { message, .. }) => {
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
}
