use super::*;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Fixture {
    _dir: tempfile::TempDir,
    reports: Reports,
    turn: evidence::Turn,
    events: Arc<std::sync::Mutex<Vec<nostr::Event>>>,
    visible: Arc<AtomicBool>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let session = uuid::Uuid::new_v4().to_string();
    let turn = evidence::Turn {
        version: 1,
        session: session.clone(),
        turn_id: uuid::Uuid::new_v4().to_string(),
        channel_id: uuid::Uuid::new_v4(),
        root_event_id: "ab".repeat(32),
        reply_event_id: "cd".repeat(32),
        event_ids: vec![],
        previous_turns: vec![],
        recovery: false,
        active: true,
    };
    evidence::write(&root.join("session.json"), &session, false).unwrap();
    evidence::write(&root.join("active.json"), &turn, false).unwrap();
    evidence::private_dir(&root.join("turns").join(&turn.turn_id).join("calls")).unwrap();
    let events = Arc::new(std::sync::Mutex::new(Vec::<nostr::Event>::new()));
    let visible = Arc::new(AtomicBool::new(true));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let stored = events.clone();
    let readable = visible.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut data = Vec::new();
            let (offset, size, path) = loop {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                data.extend_from_slice(&buf[..n]);
                let mut headers = [httparse::EMPTY_HEADER; 32];
                let mut request = httparse::Request::new(&mut headers);
                if let httparse::Status::Complete(offset) = request.parse(&data).unwrap() {
                    let size = request
                        .headers
                        .iter()
                        .find(|h| h.name.eq_ignore_ascii_case("content-length"))
                        .map(|h| {
                            std::str::from_utf8(h.value)
                                .unwrap()
                                .parse::<usize>()
                                .unwrap()
                        })
                        .unwrap();
                    break (offset, size, request.path.unwrap().to_owned());
                }
            };
            while data.len() < offset + size {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                data.extend_from_slice(&buf[..n]);
            }
            let body = if path == "/events" {
                let event: nostr::Event =
                    serde_json::from_slice(&data[offset..offset + size]).unwrap();
                event.verify().unwrap();
                stored.lock().unwrap().push(event);
                json!({"accepted":true})
            } else {
                assert_eq!(path, "/query");
                if readable.load(Ordering::SeqCst) {
                    json!(*stored.lock().unwrap())
                } else {
                    json!([])
                }
            }
            .to_string();
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len());
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let rest = RestClient {
        http: reqwest::Client::new(),
        base_url,
        keys: nostr::Keys::generate(),
        auth_tag_json: None,
    };
    Fixture {
        _dir: dir,
        reports: Reports::new(root, session, rest),
        turn,
        events,
        visible,
        server,
    }
}

#[tokio::test]
async fn literal_report_is_signed_and_read_back_with_exact_thread_routing() {
    let f = fixture().await;
    let literal = "Final contents:\n```text\nSeed: $HOME\n$(touch /should-never-execute)\n`uname` 'single' \"double\" \\ ☃\n```";
    let report: Report =
        serde_json::from_value(json!({"turn_id":f.turn.turn_id,"content":literal})).unwrap();
    let result = f.reports.report_publish(Parameters(report)).await;
    assert_ne!(result.is_error, Some(true));
    let events = f.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].content, literal);
    assert_eq!(events[0].pubkey, f.reports.rest.keys.public_key());
    let tags = events[0].tags.as_slice();
    assert!(tags
        .iter()
        .any(|t| t.as_slice() == ["h", &f.turn.channel_id.to_string()]));
    let routing = crate::queue::parse_thread_tags(&events[0]);
    assert_eq!(routing.root_event_id, Some(f.turn.root_event_id.clone()));
    assert_eq!(routing.parent_event_id, Some(f.turn.reply_event_id.clone()));
}

#[tokio::test]
async fn unresolved_delivery_retries_the_same_signed_event_across_reporter_restart() {
    let f = fixture().await;
    f.visible.store(false, Ordering::SeqCst);
    assert!(f
        .reports
        .publish(&f.turn.turn_id, "literal `report`".into())
        .await
        .unwrap_err()
        .to_string()
        .contains("unresolved"));
    let original = f.events.lock().unwrap()[0].id;
    let restarted = Reports::new(
        f.reports.root.clone(),
        f.reports.session.clone(),
        f.reports.rest.clone(),
    );
    assert!(restarted
        .publish(&f.turn.turn_id, "different content".into())
        .await
        .is_err());
    assert_eq!(f.events.lock().unwrap().len(), 1);
    // A newly signed event would now have a different timestamp and ID.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    f.visible.store(true, Ordering::SeqCst);
    let receipt = restarted
        .publish(&f.turn.turn_id, "literal `report`".into())
        .await
        .unwrap();
    assert_eq!(receipt["event_id"], original.to_hex());
    assert_eq!(receipt["readback_verified"], true);
    assert!(f.events.lock().unwrap().iter().all(|e| e.id == original));
}

#[tokio::test]
async fn old_turn_cancelled_turn_and_replaced_session_cannot_publish() {
    let f = fixture().await;
    assert!(f
        .reports
        .publish(&uuid::Uuid::new_v4().to_string(), "stale".into())
        .await
        .is_err());
    let mut cancelled = f.turn.clone();
    cancelled.active = false;
    evidence::write(&f.reports.root.join("active.json"), &cancelled, false).unwrap();
    assert!(f
        .reports
        .publish(&f.turn.turn_id, "cancelled".into())
        .await
        .is_err());
    evidence::write(&f.reports.root.join("active.json"), &f.turn, false).unwrap();
    evidence::write(
        &f.reports.root.join("session.json"),
        &uuid::Uuid::new_v4().to_string(),
        false,
    )
    .unwrap();
    assert!(f
        .reports
        .publish(&f.turn.turn_id, "replaced".into())
        .await
        .is_err());
    assert!(f.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn concurrent_identical_reports_have_one_event_identity() {
    let f = fixture().await;
    let other = Reports::new(
        f.reports.root.clone(),
        f.reports.session.clone(),
        f.reports.rest.clone(),
    );
    let (a, b) = tokio::join!(
        f.reports.publish(&f.turn.turn_id, "same".into()),
        other.publish(&f.turn.turn_id, "same".into())
    );
    assert_eq!(a.unwrap()["event_id"], b.unwrap()["event_id"]);
    let ids: std::collections::HashSet<_> = f.events.lock().unwrap().iter().map(|e| e.id).collect();
    assert_eq!(ids.len(), 1);
    let receipt: Value = evidence::read(
        &f.reports
            .root
            .join("turns")
            .join(&f.turn.turn_id)
            .join("report-receipt.json"),
    )
    .unwrap();
    assert_eq!(receipt["readback_verified"], true);
}
