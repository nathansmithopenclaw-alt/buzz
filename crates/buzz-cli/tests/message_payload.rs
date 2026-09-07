use std::process::{Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{body::Bytes, extract::State, http::StatusCode, routing::any, Json, Router};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::process::Command;

const CHANNEL: &str = "123e4567-e89b-12d3-a456-426614174000";
const PARENT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const KEY: &str = "0000000000000000000000000000000000000000000000000000000000000001";

#[derive(Default)]
struct Captured {
    requests: Vec<String>,
    events: Vec<Value>,
    fail_upload: bool,
}
type Capture = Arc<Mutex<Captured>>;

async fn respond(
    State(capture): State<Capture>,
    uri: axum::http::Uri,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    let mut capture = capture.lock().expect("capture");
    capture.requests.push(uri.path().to_owned());
    match uri.path() {
        "/events" => {
            capture
                .events
                .push(serde_json::from_slice(&body).expect("event"));
            (
                StatusCode::OK,
                Json(json!({"event_id": PARENT, "accepted": true, "message": ""})),
            )
        }
        "/query" => (
            StatusCode::OK,
            Json(json!([{"id":PARENT,"kind":9,"content":"parent","tags":[["h",CHANNEL]]}])),
        ),
        "/upload" if capture.fail_upload => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"fixture upload failure"})),
        ),
        "/upload" => (
            StatusCode::OK,
            Json(json!({"url":"https://relay.test/media/fixture.png",
            "sha256":"fixture-sha256","size":8,"type":"image/png","uploaded":0})),
        ),
        _ => (StatusCode::NOT_FOUND, Json(json!({}))),
    }
}

struct Relay {
    url: String,
    capture: Capture,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Relay {
    async fn start() -> Self {
        let capture = Capture::default();
        let app = Router::new()
            .fallback(any(respond))
            .with_state(capture.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let task = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
        Self { url, capture, task }
    }

    async fn send(&self, args: &[&str], input: &[u8], delay: Duration) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_buzz"));
        // Each process gets only the explicit fixture identity, even when a
        // developer runs this test from a managed-agent shell.
        for (name, _) in std::env::vars_os() {
            let text = name.to_string_lossy();
            if text.starts_with("BUZZ_") || text.starts_with("CLAUDE_") {
                command.env_remove(name);
            }
        }
        command
            .args([
                "--relay",
                &self.url,
                "--private-key",
                KEY,
                "messages",
                "send",
                "--channel",
                CHANNEL,
            ])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().expect("spawn CLI");
        let mut stdin = child.stdin.take().expect("stdin");
        let mut stdout = child.stdout.take().expect("stdout").take(65_537);
        let mut stderr = child.stderr.take().expect("stderr").take(65_537);
        let mut out = Vec::new();
        let mut err = Vec::new();
        let operation = async {
            let write = async {
                tokio::time::sleep(delay).await;
                if !input.is_empty() {
                    stdin.write_all(input).await.expect("write input");
                }
                drop(stdin);
            };
            let (status, stdout_result, stderr_result, _) = tokio::join!(
                child.wait(),
                stdout.read_to_end(&mut out),
                stderr.read_to_end(&mut err),
                write
            );
            stdout_result.expect("read CLI stdout");
            stderr_result.expect("read CLI stderr");
            status.expect("wait CLI")
        };
        let status = tokio::time::timeout(Duration::from_secs(10), operation)
            .await
            .expect("CLI deadline");
        assert!(
            out.len() <= 65_536 && err.len() <= 65_536,
            "bounded CLI output"
        );
        Output {
            status,
            stdout: out,
            stderr: err,
        }
    }
}

fn rejected(output: &Output) {
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).expect("JSON error");
    assert_eq!(error["error"], "user_error");
    assert_eq!(error["retryable"], false);
    assert!(error["message"]
        .as_str()
        .expect("message")
        .contains("empty or whitespace-only"));
}

#[tokio::test]
async fn blank_input_never_queries_uploads_or_publishes() {
    for content in ["", " \t\r\n", "\u{00a0}\u{2003}\u{3000}"] {
        for kind in ["9", "45001", "45003"] {
            let relay = Relay::start().await;
            let output = relay
                .send(
                    &["--content", content, "--kind", kind, "--reply-to", PARENT],
                    b"",
                    Duration::ZERO,
                )
                .await;
            rejected(&output);
            assert!(relay.capture.lock().expect("capture").requests.is_empty());
        }
    }
    for input in [b"".as_slice(), b" \n\t".as_slice()] {
        let relay = Relay::start().await;
        rejected(&relay.send(&["--content", "-"], input, Duration::ZERO).await);
        assert!(relay.capture.lock().expect("capture").requests.is_empty());
    }
}

#[tokio::test]
async fn piped_markdown_is_preserved_after_a_delayed_producer() {
    let relay = Relay::start().await;
    let content = "  indented  \n\tMarkdown `code` $literal 🐝\n";
    let output = relay
        .send(
            &["--content", "-"],
            content.as_bytes(),
            Duration::from_millis(5250),
        )
        .await;
    assert!(output.status.success(), "{output:?}");
    let capture = relay.capture.lock().expect("capture");
    assert_eq!(capture.events.len(), 1);
    assert_eq!(capture.events[0]["content"], content);
}

#[tokio::test]
async fn captionless_attachment_succeeds_and_failed_upload_never_publishes() {
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new().expect("file");
    file.write_all(b"\x89PNG\r\n\x1a\n").expect("fixture");
    let path = file.path().to_str().expect("path");
    for content in ["", "-"] {
        let relay = Relay::start().await;
        let output = relay
            .send(&["--content", content, "--file", path], b"", Duration::ZERO)
            .await;
        assert!(output.status.success(), "{output:?}");
        let capture = relay.capture.lock().expect("capture");
        assert_eq!(capture.events.len(), 1);
        assert_eq!(
            capture.events[0]["content"],
            "\n![image](https://relay.test/media/fixture.png)"
        );
    }
    let relay = Relay::start().await;
    relay.capture.lock().expect("capture").fail_upload = true;
    let output = relay
        .send(&["--content", "", "--file", path], b"", Duration::ZERO)
        .await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let capture = relay.capture.lock().expect("capture");
    assert!(!capture.requests.is_empty());
    assert!(capture.requests.iter().all(|path| path == "/upload"));
    assert!(capture.events.is_empty());
}

#[tokio::test]
async fn valid_replies_and_forum_messages_keep_their_kind_and_parent() {
    for kind in ["9", "45001", "45003"] {
        let relay = Relay::start().await;
        let output = relay
            .send(
                &["--content", "reply", "--kind", kind, "--reply-to", PARENT],
                b"",
                Duration::ZERO,
            )
            .await;
        assert!(output.status.success(), "{output:?}");
        let capture = relay.capture.lock().expect("capture");
        assert_eq!(capture.events.len(), 1);
        assert_eq!(
            capture.events[0]["kind"],
            kind.parse::<u16>().expect("kind")
        );
        if kind != "45001" {
            assert!(capture.events[0]["tags"]
                .as_array()
                .expect("tags")
                .iter()
                .any(|tag| tag[0] == "e" && tag[1] == PARENT && tag[3] == "reply"));
        }
    }
}
