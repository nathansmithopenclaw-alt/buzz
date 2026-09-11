//! Exercise the shipped CLI, its signed recovery files and real HTTP boundary.
use std::process::Output;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{body::Bytes, extract::State, http::StatusCode, routing::any, Json, Router};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::process::Command;
use uuid::Uuid;

#[derive(Default)]
struct History {
    events: Vec<Event>,
    writes: Vec<Event>,
    reject_link: bool,
    omit_link: bool,
    reject_task: bool,
    lose_task_receipt: bool,
}
type Shared = Arc<Mutex<History>>;

fn matches(event: &Event, filter: &Value) -> bool {
    if let Some(kinds) = filter["kinds"].as_array() {
        if !kinds.contains(&json!(event.kind.as_u16())) {
            return false;
        }
    }
    if let Some(ids) = filter["ids"].as_array() {
        if !ids.contains(&json!(event.id.to_hex())) {
            return false;
        }
    }
    if let Some(since) = filter["since"].as_u64() {
        if event.created_at.as_secs() < since {
            return false;
        }
    }
    for (key, values) in filter.as_object().unwrap() {
        if let Some(name) = key.strip_prefix('#') {
            if !event.tags.iter().any(|tag| {
                let tag = tag.as_slice();
                tag.first().is_some_and(|v| v == name)
                    && tag
                        .get(1)
                        .is_some_and(|v| values.as_array().unwrap().contains(&json!(v)))
            }) {
                return false;
            }
        }
    }
    true
}

async fn respond(
    State(shared): State<Shared>,
    uri: axum::http::Uri,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    let mut state = shared.lock().unwrap();
    match uri.path() {
        "/query" => {
            let filters: Vec<Value> = serde_json::from_slice(&body).unwrap();
            let mut events: Vec<_> = state
                .events
                .iter()
                .filter(|e| filters.iter().any(|filter| matches(e, filter)))
                .cloned()
                .collect();
            events.sort_by_key(|e| std::cmp::Reverse((e.created_at, e.id)));
            (StatusCode::OK, Json(json!(events)))
        }
        "/events" => {
            let event: Event = serde_json::from_slice(&body).unwrap();
            event.verify().unwrap();
            state.writes.push(event.clone());
            let kind = event.kind.as_u16();
            if (kind == 40021 && state.reject_link) || (kind == 9 && state.reject_task) {
                return (
                    StatusCode::OK,
                    Json(json!({"accepted":false,"message":"fixture rejection"})),
                );
            }
            if !(state.events.iter().any(|e| e.id == event.id) || kind == 40021 && state.omit_link)
            {
                state.events.push(event.clone());
            }
            if kind == 9 && state.lose_task_receipt {
                return (
                    StatusCode::OK,
                    Json(json!({"unknown":"accepted write without its acknowledgement"})),
                );
            }
            (
                StatusCode::OK,
                Json(json!({"accepted":true,"event_id":event.id.to_hex(),"message":""})),
            )
        }
        _ => (StatusCode::NOT_FOUND, Json(json!({}))),
    }
}

struct Fixture {
    url: String,
    history: Shared,
    server: tokio::task::JoinHandle<()>,
    cache: tempfile::TempDir,
    owner: Keys,
    executor: Keys,
    relay: Keys,
    workspace: Uuid,
    request: Uuid,
    issue: String,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    async fn start() -> Self {
        let owner = Keys::generate();
        let executor = Keys::generate();
        let relay = Keys::generate();
        let workspace = Uuid::new_v4();
        let membership = EventBuilder::new(Kind::Custom(39002), "")
            .tags([
                Tag::parse(["d", &workspace.to_string()]).unwrap(),
                Tag::parse(["p", &owner.public_key().to_hex(), "", "owner"]).unwrap(),
                Tag::parse(["p", &executor.public_key().to_hex()]).unwrap(),
            ])
            .sign_with_keys(&relay)
            .unwrap();
        let history = Arc::new(Mutex::new(History {
            events: vec![membership],
            ..Default::default()
        }));
        let app = Router::new()
            .fallback(any(respond))
            .with_state(history.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut f = Self {
            url,
            history,
            server,
            cache: tempfile::tempdir().unwrap(),
            owner,
            executor,
            relay,
            workspace,
            request: Uuid::new_v4(),
            issue: String::new(),
        };
        let output = f
            .run(&[
                "track",
                "create",
                "--channel",
                &workspace.to_string(),
                "--title",
                "assignment fixture",
                "--status",
                "todo",
                "--assignee",
                &f.owner.public_key().to_hex(),
            ])
            .await;
        let created = success(&output);
        f.issue = created["issue"]["issue_id"].as_str().unwrap().to_string();
        f.history.lock().unwrap().writes.clear();
        f
    }

    async fn run(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_buzz"));
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("BUZZ_") {
                command.env_remove(key);
            }
        }
        command
            .args(args)
            .env("BUZZ_RELAY_URL", &self.url)
            .env("BUZZ_PRIVATE_KEY", self.owner.secret_key().to_secret_hex())
            .env("BUZZ_WORKSPACE_ID", self.workspace.to_string())
            .env("BUZZ_TRACKER_CACHE_DIR", self.cache.path())
            .kill_on_drop(true);
        tokio::time::timeout(Duration::from_secs(15), command.output())
            .await
            .unwrap()
            .unwrap()
    }

    async fn assign(&self, content: &str) -> Output {
        self.assign_at(content, None).await
    }

    async fn assign_at(&self, content: &str, revision: Option<&str>) -> Output {
        let assignee = self.executor.public_key().to_hex();
        let request = self.request.to_string();
        let mut args = vec![
            "track",
            "dispatch",
            "--issue",
            &self.issue,
            "--assignee",
            &assignee,
            "--verify",
            "check the local artifact",
            "--content",
            content,
            "--request-id",
            &request,
        ];
        if let Some(revision) = revision {
            args.extend(["--if-revision", revision]);
        }
        self.run(&args).await
    }

    fn tasks(&self) -> Vec<Event> {
        self.history
            .lock()
            .unwrap()
            .writes
            .iter()
            .filter(|e| e.kind.as_u16() == 9)
            .cloned()
            .collect()
    }
}

fn success(output: &Output) -> Value {
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

#[tokio::test]
async fn workspace_task_admission_hydrates_its_message_without_blocking_issue_reads() {
    use buzz_sdk::dispatch_admission::{
        build_dispatch_admission, AdmissionBasis, DispatchAdmission,
    };
    let fixture = Fixture::start().await;
    let assignment = success(&fixture.assign("same-workspace execution").await);
    let task = fixture.tasks().pop().unwrap();
    let receipt = build_dispatch_admission(&DispatchAdmission {
        v: 1,
        workspace: fixture.workspace,
        event_id: task.id.to_hex(),
        author: task.pubkey.to_hex(),
        role: "maintainer".into(),
        observed_at: nostr::Timestamp::now().as_secs(),
        basis: AdmissionBasis::Admission,
    })
    .unwrap()
    .sign_with_keys(&fixture.relay)
    .unwrap();
    fixture.history.lock().unwrap().events.push(receipt);
    let issue = success(
        &fixture
            .run(&["track", "get", "--issue", &fixture.issue])
            .await,
    );
    assert_eq!(issue["issue"]["active_task_id"], assignment["task_id"]);
    // Retained source hydration also keeps a subsequent mutation usable.
    success(
        &fixture
            .run(&[
                "track",
                "comment",
                "--issue",
                &fixture.issue,
                "--content",
                "readback confirmed",
            ])
            .await,
    );
}

#[tokio::test]
async fn rapid_revisions_follow_parent_time_without_skipping_other_incoming_work() {
    use buzz_sdk::tracker::{
        build_tracker_issue, build_tracker_revision, IssueRevision, IssueRoot, RevisionSet,
        SetOperations, TRACKER_VERSION,
    };
    let f = Fixture::start().await;
    let root = f
        .history
        .lock()
        .unwrap()
        .events
        .iter()
        .find(|e| e.kind.as_u16() == 40020)
        .unwrap()
        .clone();
    let mut issue: IssueRoot = serde_json::from_str(&root.content).unwrap();
    let revision = IssueRevision {
        v: TRACKER_VERSION,
        op: "revise".into(),
        issue_id: issue.issue_id,
        root_event: root.id.to_hex(),
        parent_event: root.id.to_hex(),
        revision: 1,
        set: RevisionSet {
            body: Some("A reviewed parent slightly ahead of the local clock".into()),
            ..Default::default()
        },
        clear: vec![],
        relations: SetOperations::default(),
        external_refs: SetOperations::default(),
        reason: None,
        request_id: Some(Uuid::new_v4()),
        import: None,
    };
    let parent = build_tracker_revision(f.workspace, &revision)
        .unwrap()
        .custom_created_at(nostr::Timestamp::from(
            nostr::Timestamp::now().as_secs() + 10,
        ))
        .sign_with_keys(&f.owner)
        .unwrap();
    f.history.lock().unwrap().events.push(parent.clone());
    success(
        &f.run(&[
            "track",
            "update",
            "--issue",
            &f.issue,
            "--body",
            "Rapid next edit",
        ])
        .await,
    );
    success(
        &f.run(&[
            "track",
            "link",
            "--issue",
            &f.issue,
            "--type",
            "url",
            "--ref",
            "https://example.com/acceptance",
        ])
        .await,
    );
    success(&f.assign("after reviewed parent").await);
    let revisions: Vec<_> = f
        .history
        .lock()
        .unwrap()
        .writes
        .iter()
        .filter(|e| e.kind.as_u16() == 40021)
        .cloned()
        .collect();
    assert_eq!(revisions.len(), 3);
    let mut previous = parent.created_at;
    for revision in revisions {
        assert!(revision.created_at > previous);
        previous = revision.created_at;
    }
    // Advancing a signed revision must not move the read checkpoint into the
    // future and hide unrelated work arriving at the relay's current clock.
    issue.issue_id = Uuid::new_v4();
    issue.request_id = Some(Uuid::new_v4());
    issue.title = "Different incoming issue at current time".into();
    issue.identifier = "TEST-999".into();
    let incoming = build_tracker_issue(f.workspace, &issue)
        .unwrap()
        .sign_with_keys(&f.owner)
        .unwrap();
    f.history.lock().unwrap().events.push(incoming);
    let read = success(
        &f.run(&["track", "get", "--issue", &issue.issue_id.to_string()])
            .await,
    );
    assert_eq!(read["issue"]["title"], issue.title);
}

#[tokio::test]
async fn issue_link_precedes_execution_and_repeat_does_not_publish() {
    let f = Fixture::start().await;
    let first = success(&f.assign("reviewed scope").await);
    let writes = f.history.lock().unwrap().writes.clone();
    assert_eq!(
        writes.iter().map(|e| e.kind.as_u16()).collect::<Vec<_>>(),
        vec![40021, 9]
    );
    let link: Value = serde_json::from_str(&writes[0].content).unwrap();
    assert!(link["set"].get("assignee").is_none_or(Value::is_null));
    assert!(link["external_refs"]["add"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["ref"] == writes[1].id.to_hex()));
    let second = success(&f.assign("reviewed scope").await);
    assert_eq!(second["task_event_id"], first["task_event_id"]);
    assert_eq!(second["replayed"], true);
    assert_eq!(f.history.lock().unwrap().writes.len(), 2);
}

#[tokio::test]
async fn rejected_or_unreadable_link_never_publishes_execution() {
    for omit in [false, true] {
        let f = Fixture::start().await;
        {
            let mut h = f.history.lock().unwrap();
            h.omit_link = omit;
            h.reject_link = !omit;
        }
        let output = f.assign("reviewed scope").await;
        assert!(!output.status.success());
        let recovery: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(recovery["request_id"], f.request.to_string());
        assert_eq!(recovery["accepted"], false);
        assert!(f.tasks().is_empty());
    }
}

#[tokio::test]
async fn restart_after_task_rejection_resends_exact_bytes_and_keeps_one_link() {
    let f = Fixture::start().await;
    f.history.lock().unwrap().reject_task = true;
    assert!(!f.assign("reviewed scope").await.status.success());
    let original = f.tasks()[0].clone();
    assert!(!f.assign("changed scope").await.status.success());
    assert_eq!(f.tasks().len(), 1);
    f.history.lock().unwrap().reject_task = false;
    success(&f.assign("reviewed scope").await);
    assert_eq!(f.tasks(), vec![original.clone(), original]);
    assert_eq!(
        f.history
            .lock()
            .unwrap()
            .writes
            .iter()
            .filter(|e| e.kind.as_u16() == 40021)
            .count(),
        1
    );
}

#[tokio::test]
async fn lost_task_acknowledgement_is_resolved_from_accepted_history() {
    let f = Fixture::start().await;
    f.history.lock().unwrap().lose_task_receipt = true;
    assert!(!f.assign("reviewed scope").await.status.success());
    f.history.lock().unwrap().lose_task_receipt = false;
    assert_eq!(success(&f.assign("reviewed scope").await)["replayed"], true);
    assert_eq!(f.tasks().len(), 1);
}

#[tokio::test]
async fn changed_issue_requires_explicit_latest_revision_before_recovery() {
    let f = Fixture::start().await;
    f.history.lock().unwrap().reject_task = true;
    assert!(!f.assign("reviewed scope").await.status.success());
    let update = success(
        &f.run(&[
            "track",
            "update",
            "--issue",
            &f.issue,
            "--body",
            "A second client changed the issue context.",
        ])
        .await,
    );
    f.history.lock().unwrap().reject_task = false;
    assert!(!f.assign("reviewed scope").await.status.success());
    assert_eq!(f.tasks().len(), 1);
    success(
        &f.assign_at(
            "reviewed scope",
            update["issue"]["revision_event_id"].as_str(),
        )
        .await,
    );
    assert_eq!(f.tasks().len(), 2);
    assert_eq!(f.tasks()[0], f.tasks()[1]);
}

#[tokio::test]
async fn rejected_link_recovers_with_original_revision_and_signed_bytes() {
    let f = Fixture::start().await;
    let original = success(&f.run(&["track", "get", "--issue", &f.issue]).await);
    let original_tip = original["issue"]["revision_event_id"].as_str();
    f.history.lock().unwrap().reject_link = true;
    assert!(!f
        .assign_at("reviewed scope", original_tip)
        .await
        .status
        .success());
    let original_link = f.history.lock().unwrap().writes[0].clone();
    f.history.lock().unwrap().reject_link = false;
    success(&f.assign_at("reviewed scope", original_tip).await);
    assert_eq!(f.history.lock().unwrap().writes[1], original_link);
    success(&f.assign_at("reviewed scope", original_tip).await);
    assert_eq!(f.tasks().len(), 1);
}

#[tokio::test]
async fn corrupted_recovery_is_preserved_and_never_replaced_with_new_assignment() {
    let f = Fixture::start().await;
    f.history.lock().unwrap().reject_task = true;
    assert!(!f.assign("reviewed scope").await.status.success());
    let directory = std::fs::read_dir(f.cache.path().join("dispatch-assignments-v1"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let path = directory.join(format!("{}.json", f.request));
    std::fs::write(&path, b"retained but corrupted").unwrap();
    let output = f.assign("reviewed scope").await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("recovery storage failed"));
    assert_eq!(std::fs::read(path).unwrap(), b"retained but corrupted");
    assert_eq!(f.tasks().len(), 1);
}

#[tokio::test]
async fn another_process_cannot_enter_the_same_issue_assignment() {
    let f = Fixture::start().await;
    success(&f.assign("reviewed scope").await);
    let directory = std::fs::read_dir(f.cache.path().join("dispatch-assignments-v1"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.join(format!("{}.lock", f.issue)))
        .unwrap();
    fs2::FileExt::try_lock_exclusive(&lock).unwrap();
    let blocked = f.assign("reviewed scope").await;
    assert_eq!(blocked.status.code(), Some(5));
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("another assignment"));
    assert_eq!(f.tasks().len(), 1);
    drop(lock);
    success(&f.assign("reviewed scope").await);
}
