//! HTTP boundary tests use freshly signed disposable identities and an isolated relay.
use super::*;
use buzz_sdk::{
    dispatch_automation::{AutomationControls, AutomationMode, AutomationRoute},
    dispatch_record::{build_dispatch_record, DispatchRecord, DispatchRecordPayload},
    tracker::{build_tracker_issue, IssueRoot, IssueStatus, TRACKER_VERSION},
};
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
};
use uuid::Uuid;
#[derive(Default)]
struct RelayState {
    events: Vec<Event>,
    submissions: Vec<Event>,
    fail_kind: Option<u16>,
    omit_admissions: bool,
    page_limit: Option<usize>,
    malformed_query: bool,
}
struct MockRelay {
    state: Arc<Mutex<RelayState>>,
    url: String,
    stop: Arc<std::sync::atomic::AtomicBool>,
}
impl Drop for MockRelay {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed)
    }
}
impl MockRelay {
    fn new(events: Vec<Event>, issuer: Keys) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(RelayState {
            events,
            ..Default::default()
        }));
        let server = state.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopping = stop.clone();
        std::thread::spawn(move || {
            while !stopping.load(std::sync::atomic::Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = vec![];
                let mut chunk = [0; 4096];
                let (header_end, length) = loop {
                    let count = stream.read(&mut chunk).unwrap();
                    if count == 0 {
                        return;
                    };
                    bytes.extend_from_slice(&chunk[..count]);
                    if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..end]);
                        let length = header
                            .lines()
                            .find_map(|line| {
                                line.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|length| length.trim().parse::<usize>().unwrap())
                            })
                            .unwrap();
                        break (end + 4, length);
                    }
                };
                while bytes.len() < header_end + length {
                    let count = stream.read(&mut chunk).unwrap();
                    bytes.extend_from_slice(&chunk[..count])
                }
                let body: Value =
                    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
                let query =
                    String::from_utf8_lossy(&bytes[..header_end]).starts_with("POST /query ");
                let mut data = server.lock().unwrap();
                seal_sources(&mut data, &issuer);
                let response = if query {
                    let mut found = vec![];
                    for filter in body.as_array().unwrap() {
                        let mut matching = data
                            .events
                            .iter()
                            .filter(|event| matches_filter(event, filter))
                            .cloned()
                            .collect::<Vec<_>>();
                        matching
                            .sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
                        matching.truncate(
                            data.page_limit
                                .unwrap_or(filter["limit"].as_u64().unwrap_or(500) as usize),
                        );
                        found.extend(matching)
                    }
                    if data.malformed_query {
                        json!([{ "malformed": true }])
                    } else {
                        json!(found)
                    }
                } else {
                    let event: Event = serde_json::from_value(body).unwrap();
                    event.verify().unwrap();
                    data.submissions.push(event.clone());
                    if !data.events.iter().any(|old| old.id == event.id) {
                        data.events.push(event.clone())
                    };
                    let fail = data.fail_kind == Some(event.kind.as_u16());
                    if fail {
                        data.fail_kind = None
                    };
                    json!({"event_id":event.id.to_hex(),"accepted":!fail,"message":if fail {"simulated lost acknowledgement"}else{"stored"}})
                };
                drop(data);
                let body = response.to_string();
                let _=write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",body.len(),body);
            }
        });
        Self { state, url, stop }
    }
}
fn seal_sources(state: &mut RelayState, issuer: &Keys) {
    use buzz_sdk::dispatch_admission::{
        build_dispatch_admission, requires_dispatch_event_admission, AdmissionBasis,
        DispatchAdmission,
    };
    if state.omit_admissions {
        return;
    }
    let sealed = state
        .events
        .iter()
        .filter(|event| event.kind.as_u16() == 40027)
        .filter_map(|event| serde_json::from_str::<DispatchAdmission>(&event.content).ok())
        .map(|value| value.event_id)
        .collect::<std::collections::BTreeSet<_>>();
    let sources = state
        .events
        .iter()
        .filter(|event| {
            requires_dispatch_event_admission(event) && !sealed.contains(&event.id.to_hex())
        })
        .cloned()
        .collect::<Vec<_>>();
    for source in sources {
        let workspace = source
            .tags
            .iter()
            .find_map(|tag| {
                let t = tag.as_slice();
                (t[0] == "h").then(|| t[1].parse::<Uuid>().ok()).flatten()
            })
            .unwrap();
        let membership = state
            .events
            .iter()
            .filter(|event| event.kind.as_u16() == 39002)
            .max_by_key(|event| event.created_at)
            .unwrap();
        let role = membership.tags.iter().find_map(|tag| {
            let t = tag.as_slice();
            (t[0] == "p" && t[1] == source.pubkey.to_hex())
                .then(|| t.get(3).map(String::as_str).unwrap_or("member"))
        });
        let Some(role) = role else { continue };
        let receipt = DispatchAdmission {
            v: 1,
            workspace,
            event_id: source.id.to_hex(),
            author: source.pubkey.to_hex(),
            role: if matches!(role, "owner" | "admin" | "bot") {
                "maintainer"
            } else {
                "member"
            }
            .into(),
            observed_at: Timestamp::now().as_secs(),
            basis: AdmissionBasis::Admission,
        };
        state.events.push(
            build_dispatch_admission(&receipt)
                .unwrap()
                .sign_with_keys(issuer)
                .unwrap(),
        );
    }
}
fn matches_filter(event: &Event, filter: &Value) -> bool {
    for (key, value) in filter.as_object().unwrap() {
        match key.as_str() {
            "kinds"
                if !value
                    .as_array()
                    .unwrap()
                    .contains(&json!(event.kind.as_u16())) =>
            {
                return false;
            }
            "ids"
                if !value
                    .as_array()
                    .unwrap()
                    .contains(&json!(event.id.to_hex())) =>
            {
                return false;
            }
            "since" if event.created_at.as_secs() < value.as_u64().unwrap() => {
                return false;
            }
            "until" => {
                let until = value.as_u64().unwrap();
                if event.created_at.as_secs() > until
                    || (event.created_at.as_secs() == until
                        && filter["before_id"]
                            .as_str()
                            .is_some_and(|before| event.id.to_hex().as_str() <= before))
                {
                    return false;
                }
            }
            key if key.starts_with('#')
                && !event.tags.iter().any(|tag| {
                    let tag = tag.as_slice();
                    tag.first().map(String::as_str) == Some(&key[1..])
                        && tag
                            .get(1)
                            .is_some_and(|tag| value.as_array().unwrap().contains(&json!(tag)))
                }) =>
            {
                return false;
            }
            _ => {}
        }
    }
    true
}
struct Fixture {
    relay: MockRelay,
    service: Keys,
    owner: Keys,
    executor: Keys,
    channel: Uuid,
    issue: Uuid,
    approval: DispatchRecord,
    config: config::DispatcherConfig,
    _directory: tempfile::TempDir,
    clock: u64,
}
impl Fixture {
    fn new() -> Self {
        let service = Keys::generate();
        let owner = Keys::generate();
        let executor = Keys::generate();
        let issuer = Keys::generate();
        let channel = Uuid::new_v4();
        let issue = Uuid::new_v4();
        let clock = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 30;
        let membership = EventBuilder::new(Kind::Custom(39002), "")
            .tags([
                Tag::parse(["d", &channel.to_string()]).unwrap(),
                Tag::parse(["p", &owner.public_key().to_hex(), "", "owner"]).unwrap(),
                Tag::parse(["p", &service.public_key().to_hex(), "", "member"]).unwrap(),
                Tag::parse(["p", &executor.public_key().to_hex(), "", "member"]).unwrap(),
            ])
            .sign_with_keys(&issuer)
            .unwrap();
        let root = IssueRoot {
            v: TRACKER_VERSION,
            op: "create".into(),
            issue_id: issue,
            identifier: "TEST-1".into(),
            title: "Reviewable bounded work".into(),
            body: "Verify the isolated sample and report evidence.".into(),
            status: IssueStatus::Todo,
            priority: None,
            assignee: Some(owner.public_key().to_hex()),
            project_id: None,
            due: None,
            estimate_minutes: None,
            home_channel_id: Some(channel),
            source_event_id: None,
            workflow: "default".into(),
            labels: vec!["testing".into()],
            external_refs: vec![],
            request_id: None,
            import: None,
        };
        let root = build_tracker_issue(channel, &root)
            .unwrap()
            .custom_created_at(Timestamp::from(clock))
            .sign_with_keys(&owner)
            .unwrap();
        let capacity = DispatchRecord {
            v: 1,
            record_id: Uuid::new_v4(),
            revision: 0,
            parent_event: None,
            request_id: Uuid::new_v4(),
            archived: false,
            payload: DispatchRecordPayload::Capacity {
                executor: executor.public_key().to_hex(),
                max_active: 1,
                capabilities: vec!["testing".into()],
                paused: false,
            },
        };
        let capacity = build_dispatch_record(channel, &capacity)
            .unwrap()
            .custom_created_at(Timestamp::from(clock))
            .sign_with_keys(&owner)
            .unwrap();
        let approval = DispatchRecord {
            v: 1,
            record_id: Uuid::new_v4(),
            revision: 0,
            parent_event: None,
            request_id: Uuid::new_v4(),
            archived: false,
            payload: DispatchRecordPayload::Automation {
                controls: AutomationControls {
                    service_id: "test-dispatcher".into(),
                    service_pubkey: service.public_key().to_hex(),
                    mode: AutomationMode::Preview,
                    expires_at: None,
                    max_actions_total: 1,
                    max_actions_per_tick: 1,
                    issue_ids: vec![issue],
                    capture_channels: vec![],
                    routing: vec![AutomationRoute {
                        label: "testing".into(),
                        executor: executor.public_key().to_hex(),
                        channel,
                    }],
                    notify_triage: false,
                    unassigned_after_secs: None,
                    notification_channel: channel,
                },
            },
        };
        let approved = build_dispatch_record(channel, &approval)
            .unwrap()
            .custom_created_at(Timestamp::from(clock))
            .sign_with_keys(&owner)
            .unwrap();
        let relay = MockRelay::new(vec![membership, root, capacity, approved], issuer);
        let directory = tempfile::tempdir().unwrap();
        let config:config::DispatcherConfig=serde_json::from_value(json!({"remote_controls":true,"relay_url":relay.url,"service_id":"test-dispatcher","stream_id":channel,"poll_interval_secs":30,"monitored_channels":[channel],"default_channel":channel,"routing":[{"label":"testing","owner":executor.public_key().to_hex(),"channel":channel}],"thresholds":[],"roles":{owner.public_key().to_hex():"maintainer",service.public_key().to_hex():"member"},"decision_surface":{"channel":channel},"liveness":{"channel":channel,"stale_after_secs":300,"signer_allowlist":[service.public_key().to_hex()]},"state_dir":directory.path()})).unwrap();
        Self {
            relay,
            service,
            owner,
            executor,
            channel,
            issue,
            approval,
            config,
            _directory: directory,
            clock,
        }
    }
    fn mode(&mut self, mode: AutomationMode) {
        let mut state = self.relay.state.lock().unwrap();
        let latest = state
            .events
            .iter()
            .filter(|event| event.kind.as_u16() == 40026)
            .filter(|event| {
                serde_json::from_str::<DispatchRecord>(&event.content)
                    .is_ok_and(|value| value.record_id == self.approval.record_id)
            })
            .max_by_key(|event| event.created_at)
            .unwrap();
        self.approval.parent_event = Some(latest.id.to_hex());
        self.approval.revision += 1;
        self.approval.request_id = Uuid::new_v4();
        if let DispatchRecordPayload::Automation { controls } = &mut self.approval.payload {
            controls.mode = mode;
            controls.expires_at = Some(self.clock + 3600)
        };
        self.clock += 1;
        state.events.push(
            build_dispatch_record(self.channel, &self.approval)
                .unwrap()
                .custom_created_at(Timestamp::from(self.clock))
                .sign_with_keys(&self.owner)
                .unwrap(),
        )
    }
    async fn tick(&self) -> Result<(), DispatchError> {
        let relay =
            relay_http::RelayHttp::new(&self.relay.url, self.service.clone(), None).unwrap();
        let emitter = emit::Emitter::new(&relay, &self.config, Uuid::new_v4());
        let mut store = projection::ProjectionStore::open(&self.config.state_dir).unwrap();
        run::tick(
            &self.config,
            &relay,
            &emitter,
            &self.config.authorization(&relay.pubkey_hex()),
            &mut store,
        )
        .await
    }
}
#[tokio::test]
async fn preview_then_one_bounded_assignment_preserves_owner_and_survives_restart() {
    let mut fixture = Fixture::new();
    fixture.tick().await.unwrap();
    assert!(fixture
        .relay
        .state
        .lock()
        .unwrap()
        .submissions
        .iter()
        .all(|event| event.kind.as_u16() == 40024));
    fixture.mode(AutomationMode::Active);
    fixture.tick().await.unwrap();
    fixture.tick().await.unwrap();
    let state = fixture.relay.state.lock().unwrap();
    let tasks = state
        .submissions
        .iter()
        .filter(|event| event.kind.as_u16() == 9)
        .collect::<Vec<_>>();
    assert_eq!(tasks.len(), 1);
    let task = buzz_sdk::task_envelope::TaskEnvelopeEvent::from_event(tasks[0]).unwrap();
    assert_eq!(
        task.envelope.assignee,
        fixture.executor.public_key().to_hex()
    );
    assert_eq!(
        task.envelope.authority.unwrap().reviewer,
        Some(fixture.owner.public_key().to_hex())
    );
    let auth = fixture
        .config
        .authorization(&fixture.service.public_key().to_hex());
    let fold = buzz_sdk::tracker::fold_tracker_events(
        state.events.iter().filter_map(|event| {
            buzz_sdk::tracker::parse_tracker_event(event, u64::MAX - 1000).ok()
        }),
        &auth,
    );
    let issue = fold.issues.get(&(fixture.channel, fixture.issue)).unwrap();
    assert_eq!(issue.assignee, Some(fixture.owner.public_key().to_hex()));
    assert!(issue
        .external_refs
        .iter()
        .any(|reference| reference.kind == "task_event"
            && reference.reference == tasks[0].id.to_hex()));
    assert_eq!(
        rollout_budget::requests(&fixture.config.state_dir, fixture.approval.record_id)
            .unwrap()
            .len(),
        1
    );
}
#[tokio::test]
async fn uncertain_link_is_held_while_paused_and_resumes_exactly_once() {
    let mut fixture = Fixture::new();
    fixture.mode(AutomationMode::Active);
    fixture.relay.state.lock().unwrap().fail_kind = Some(40021);
    assert!(fixture.tick().await.is_err());
    assert!(!fixture
        .relay
        .state
        .lock()
        .unwrap()
        .submissions
        .iter()
        .any(|event| event.kind.as_u16() == 9));
    fixture.mode(AutomationMode::Paused);
    fixture.tick().await.unwrap();
    assert!(!fixture
        .relay
        .state
        .lock()
        .unwrap()
        .submissions
        .iter()
        .any(|event| event.kind.as_u16() == 9));
    fixture.mode(AutomationMode::Active);
    fixture.tick().await.unwrap();
    fixture.tick().await.unwrap();
    let state = fixture.relay.state.lock().unwrap();
    assert_eq!(
        state
            .submissions
            .iter()
            .filter(|event| event.kind.as_u16() == 9)
            .count(),
        1
    );
    let links = state
        .submissions
        .iter()
        .filter(|event| event.kind.as_u16() == 40021)
        .map(|event| event.id)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(links.len(), 1);
}

#[tokio::test]
async fn expired_approval_and_demoted_approver_produce_no_work_effects() {
    let mut fixture = Fixture::new();
    fixture.mode(AutomationMode::Active);
    if let DispatchRecordPayload::Automation { controls } = &mut fixture.approval.payload {
        controls.expires_at = Some(fixture.clock + 1)
    }
    // Replace the just-written active revision with an already expired signed approval.
    {
        let mut state = fixture.relay.state.lock().unwrap();
        state.events.retain(|event| {
            !(event.kind.as_u16() == 40026
                && serde_json::from_str::<DispatchRecord>(&event.content).is_ok_and(|record| {
                    record.record_id == fixture.approval.record_id && record.revision == 1
                }))
        });
        state.events.push(
            build_dispatch_record(fixture.channel, &fixture.approval)
                .unwrap()
                .custom_created_at(Timestamp::from(fixture.clock))
                .sign_with_keys(&fixture.owner)
                .unwrap(),
        );
    }
    fixture.tick().await.unwrap();
    assert!(fixture
        .relay
        .state
        .lock()
        .unwrap()
        .submissions
        .iter()
        .all(|event| event.kind.as_u16() == 40024));
    // Host configuration cannot keep a demoted approver's maintainer privileges.
    let mut membership = fixture
        .relay
        .state
        .lock()
        .unwrap()
        .events
        .iter()
        .find(|event| event.kind.as_u16() == 39002)
        .unwrap()
        .clone();
    let tags = membership
        .tags
        .iter()
        .map(|tag| {
            let mut values = tag.as_slice().to_vec();
            if values.first().map(String::as_str) == Some("p")
                && values.get(1) == Some(&fixture.owner.public_key().to_hex())
            {
                values[3] = "member".into()
            };
            Tag::parse(values).unwrap()
        })
        .collect::<Vec<_>>();
    membership = EventBuilder::new(Kind::Custom(39002), "")
        .tags(tags)
        .sign_with_keys(&Keys::generate())
        .unwrap();
    // This test isolates the intersection of host and current membership roles.
    fixture
        .relay
        .state
        .lock()
        .unwrap()
        .events
        .retain(|event| event.kind.as_u16() != 39002);
    fixture.relay.state.lock().unwrap().events.push(membership);
    fixture.mode(AutomationMode::Active);
    fixture.tick().await.unwrap();
    assert!(fixture
        .relay
        .state
        .lock()
        .unwrap()
        .submissions
        .iter()
        .all(|event| event.kind.as_u16() == 40024));
}

#[tokio::test]
async fn one_executor_slot_prevents_two_assignments_in_one_cycle() {
    let mut fixture = Fixture::new();
    let second = Uuid::new_v4();
    {
        let mut state = fixture.relay.state.lock().unwrap();
        let original = state
            .events
            .iter()
            .find(|event| event.kind.as_u16() == 40020)
            .unwrap();
        let mut root: IssueRoot = serde_json::from_str(&original.content).unwrap();
        root.issue_id = second;
        root.identifier = "TEST-2".into();
        root.title = "Second candidate".into();
        state.events.push(
            build_tracker_issue(fixture.channel, &root)
                .unwrap()
                .custom_created_at(Timestamp::from(fixture.clock))
                .sign_with_keys(&fixture.owner)
                .unwrap(),
        );
        if let DispatchRecordPayload::Automation { controls } = &mut fixture.approval.payload {
            controls.issue_ids.push(second);
            controls.max_actions_total = 2;
            controls.max_actions_per_tick = 2
        }
        state.events.retain(|event| {
            !(event.kind.as_u16() == 40026
                && serde_json::from_str::<DispatchRecord>(&event.content)
                    .is_ok_and(|record| record.record_id == fixture.approval.record_id))
        });
        state.events.push(
            build_dispatch_record(fixture.channel, &fixture.approval)
                .unwrap()
                .custom_created_at(Timestamp::from(fixture.clock))
                .sign_with_keys(&fixture.owner)
                .unwrap(),
        );
    }
    fixture.mode(AutomationMode::Active);
    fixture.tick().await.unwrap();
    fixture.tick().await.unwrap();
    assert_eq!(
        fixture
            .relay
            .state
            .lock()
            .unwrap()
            .submissions
            .iter()
            .filter(|event| event.kind.as_u16() == 9)
            .count(),
        1
    );
}

#[tokio::test]
async fn unsealed_history_never_activates_an_executor() {
    let mut f = Fixture::new();
    f.relay.state.lock().unwrap().omit_admissions = true;
    f.mode(AutomationMode::Active);
    let error = f.tick().await.unwrap_err();
    assert!(error.to_string().contains("historical authorization"));
    assert!(f
        .relay
        .state
        .lock()
        .unwrap()
        .submissions
        .iter()
        .all(|event| event.kind.as_u16() != 9));
}

#[tokio::test]
async fn incomplete_http_history_cannot_be_treated_as_a_complete_refresh() {
    let author = Keys::generate();
    let events = (0..201)
        .map(|index| {
            EventBuilder::new(Kind::Custom(1), format!("history-{index}"))
                .custom_created_at(Timestamp::from(1000 - index))
                .sign_with_keys(&author)
                .unwrap()
        })
        .collect();
    let server = MockRelay::new(events, Keys::generate());
    server.state.lock().unwrap().page_limit = Some(1);
    let relay = crate::relay_http::RelayHttp::new(&server.url, author, None).unwrap();
    let result = crate::run::query_paginated_events(&relay, json!({"kinds":[1]})).await;
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("page budget exhausted"));
    server.state.lock().unwrap().malformed_query = true;
    let result = crate::run::query_paginated_events(&relay, json!({"kinds":[1]})).await;
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("could not be decoded"));
    assert!(server.state.lock().unwrap().submissions.is_empty());
}
