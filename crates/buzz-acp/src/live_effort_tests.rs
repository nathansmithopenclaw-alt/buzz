use super::*;
use crate::acp::AcpClient;
use crate::pool::{OwnedAgent, SessionState, TaskMeta};
use crate::scope::SessionScope;
use std::collections::HashMap;

fn config(value: &str) -> Value {
    json!({"configOptions":[{"id":"native-reasoning","category":"thought_level","type":"select",
        "currentValue":value,"options":[{"value":"low"},{"value":"high"}]}],"relayUrl":"ws://test.invalid"})
}

async fn agent(scope: &SessionScope, file: &std::path::Path, behavior: &str) -> OwnedAgent {
    let script = r#"import json,sys,time
for line in sys.stdin:
 r=json.loads(line)
 with open(sys.argv[1],'a') as f: f.write(json.dumps(r)+'\n')
 if sys.argv[2]=='hang': time.sleep(60)
 if sys.argv[2]=='reject':
  print(json.dumps({'jsonrpc':'2.0','id':r['id'],'error':{'code':-32602,'message':'unsupported effort'}}),flush=True)
 else:
  value=r['params']['value'] if sys.argv[2]=='apply' else 'low'
  opts=[{'id':'native-reasoning','category':'thought_level','type':'select','currentValue':value,'options':[{'value':'low'},{'value':'high'}]}]
  print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':{'configOptions':opts}}),flush=True)
"#;
    let acp = AcpClient::spawn(
        "python3",
        &[
            "-u".into(),
            "-c".into(),
            script.into(),
            file.to_string_lossy().to_string(),
            behavior.into(),
        ],
        &[],
        false,
    )
    .await
    .unwrap();
    let mut state = SessionState::default();
    state.sessions.insert(scope.clone(), "same-session".into());
    state.configs.insert(scope.clone(), config("low"));
    state.turn_counts.insert(scope.clone(), 3);
    OwnedAgent {
        index: 0,
        acp,
        state,
        model_capabilities: None,
        desired_model: None,
        model_overridden: false,
        desired_model_request_id: None,
        desired_model_pending_ack: false,
        startup_effort: Some("low".into()),
        agent_name: "effort-test".into(),
        goose_system_prompt_supported: None,
        protocol_version: 2,
    }
}

fn request(channel: Uuid, session: &str, effort: &str) -> Value {
    json!({"type":"switch_effort","requestId":Uuid::new_v4(),"channelId":channel,
        "sessionId":session,"effort":effort})
}

fn status(observer: &ObserverHandle) -> String {
    observer
        .snapshot()
        .iter()
        .rev()
        .find(|e| e.kind == "control_result")
        .unwrap()
        .payload["status"]
        .as_str()
        .unwrap()
        .into()
}

#[tokio::test]
async fn applies_native_rpc_to_same_session_and_preserves_sibling_defaults_and_history() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("rpc");
    let channel = Uuid::new_v4();
    let scope = SessionScope::Thread {
        channel_id: channel,
        root_event_id: "a".repeat(64),
    };
    let sibling = SessionScope::Thread {
        channel_id: channel,
        root_event_id: "b".repeat(64),
    };
    let mut worker = agent(&scope, &file, "apply").await;
    worker
        .state
        .sessions
        .insert(sibling.clone(), "sibling-session".into());
    worker.state.configs.insert(sibling.clone(), config("low"));
    let mut pool = AgentPool::from_slots(vec![Some(worker)]);
    let observer = ObserverHandle::in_process();
    let pick = request(channel, "same-session", "high");
    pool.queue_live_effort(&pick, Some(&observer));
    assert_eq!(status(&observer), "queued");
    assert!(
        pool.try_claim(Some(&scope)).is_none(),
        "next turn must wait for effort"
    );
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(status(&observer), "applied");
    let worker = pool.try_claim(Some(&scope)).unwrap();
    assert_eq!(worker.state.sessions[&scope], "same-session");
    assert_eq!(worker.state.turn_counts[&scope], 3);
    assert_eq!(worker.startup_effort.as_deref(), Some("low"));
    assert_eq!(
        worker.state.configs[&scope]["configOptions"][0]["currentValue"],
        "high"
    );
    assert_eq!(
        worker.state.configs[&sibling]["configOptions"][0]["currentValue"],
        "low"
    );
    let captured = observer
        .snapshot()
        .into_iter()
        .find(|e| e.kind == "session_config_captured")
        .unwrap();
    assert_eq!(captured.session_id.as_deref(), Some("same-session"));
    let rpc: Value = serde_json::from_str(std::fs::read_to_string(&file).unwrap().trim()).unwrap();
    assert_eq!(rpc["method"], "session/set_config_option");
    assert_eq!(
        rpc["params"],
        json!({"sessionId":"same-session","configId":"native-reasoning","value":"high"})
    );
    pool.return_agent(worker);
    pool.queue_live_effort(&pick, Some(&observer));
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(
        std::fs::read_to_string(file).unwrap().lines().count(),
        1,
        "replay cannot execute twice"
    );
}

#[tokio::test]
async fn busy_turn_queues_without_cancelling_then_applies_before_next_claim() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("rpc");
    let channel = Uuid::new_v4();
    let scope = SessionScope::Conversation {
        channel_id: channel,
    };
    let worker = agent(&scope, &file, "apply").await;
    let mut pool = AgentPool::from_slots(vec![None]);
    let observer = ObserverHandle::in_process();
    let task = tokio::spawn(std::future::pending::<()>());
    let id = task.id();
    let (control_tx, mut control_rx) = tokio::sync::oneshot::channel();
    pool.task_map_mut().insert(
        id,
        TaskMeta {
            agent_index: 0,
            channel_id: Some(channel),
            scope: Some(scope.clone()),
            turn_id: "busy".into(),
            recoverable_batch: None,
            control_tx: Some(control_tx),
            steer_tx: None,
            successful_steer_deliveries: HashMap::new(),
        },
    );
    pool.queue_live_effort(&request(channel, "same-session", "high"), Some(&observer));
    pool.apply_pending_effort(Some(&observer)).await;
    assert!(
        !file.exists(),
        "must not write while a response owns the stream"
    );
    assert!(
        matches!(
            control_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ),
        "must not cancel the response"
    );
    pool.task_map_mut().remove(&id);
    task.abort();
    pool.return_agent(worker);
    assert!(pool.try_claim(Some(&scope)).is_none());
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(status(&observer), "applied");
    assert!(pool.try_claim(Some(&scope)).is_some());
}

#[tokio::test]
async fn rejects_unsupported_and_stale_targets_without_any_rpc() {
    for (session, effort, expected) in [
        ("same-session", "ultra", "unsupported"),
        ("expired-session", "high", "stale_session"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("rpc");
        let channel = Uuid::new_v4();
        let scope = SessionScope::Conversation {
            channel_id: channel,
        };
        let mut pool = AgentPool::from_slots(vec![Some(agent(&scope, &file, "apply").await)]);
        let observer = ObserverHandle::in_process();
        pool.queue_live_effort(&request(channel, session, effort), Some(&observer));
        pool.apply_pending_effort(Some(&observer)).await;
        assert_eq!(status(&observer), expected);
        assert!(!file.exists());
        assert_eq!(
            pool.agents_mut()[0].as_ref().unwrap().state.configs[&scope],
            config("low")
        );
    }
}

#[tokio::test]
async fn adapter_rejection_preserves_session_and_mismatched_ack_is_unconfirmed() {
    for (behavior, expected) in [("reject", "rejected"), ("mismatch", "unconfirmed")] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("rpc");
        let channel = Uuid::new_v4();
        let scope = SessionScope::Conversation {
            channel_id: channel,
        };
        let mut pool = AgentPool::from_slots(vec![Some(agent(&scope, &file, behavior).await)]);
        let observer = ObserverHandle::in_process();
        pool.queue_live_effort(&request(channel, "same-session", "high"), Some(&observer));
        pool.apply_pending_effort(Some(&observer)).await;
        assert_eq!(status(&observer), expected);
        let worker = pool.try_claim(Some(&scope)).unwrap();
        assert_eq!(worker.state.sessions[&scope], "same-session");
        assert_eq!(
            worker.state.configs[&scope]["configOptions"][0]["currentValue"],
            "low"
        );
    }
}

#[tokio::test]
async fn bounds_pending_controls_and_releases_expired_claim_fences() {
    let mut pool = AgentPool::from_slots(vec![]);
    let observer = ObserverHandle::in_process();
    for _ in 0..CAPACITY {
        pool.queue_live_effort(&request(Uuid::new_v4(), "session", "high"), Some(&observer));
    }
    pool.queue_live_effort(
        &request(Uuid::new_v4(), "overflow", "high"),
        Some(&observer),
    );
    assert_eq!(status(&observer), "busy");
    assert_eq!(pool.live_effort.pending.len(), CAPACITY);
    for item in &mut pool.live_effort.pending {
        item.received = Instant::now() - EXPIRY - Duration::from_secs(1);
    }
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(status(&observer), "expired");
    assert!(pool.live_effort.pending.is_empty());
}

#[tokio::test]
async fn timeout_retires_the_stream_without_claiming_application() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("rpc");
    let channel = Uuid::new_v4();
    let scope = SessionScope::Conversation {
        channel_id: channel,
    };
    let mut pool = AgentPool::from_slots(vec![Some(agent(&scope, &file, "hang").await)]);
    let observer = ObserverHandle::in_process();
    pool.queue_live_effort(&request(channel, "same-session", "high"), Some(&observer));
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(status(&observer), "unconfirmed");
    assert!(pool.agents_mut()[0].is_none());
}

#[tokio::test]
async fn pending_edit_holds_exact_owner_without_blocking_another_idle_worker() {
    let dir = tempfile::tempdir().unwrap();
    let scope = SessionScope::Conversation {
        channel_id: Uuid::new_v4(),
    };
    let sibling = SessionScope::Conversation {
        channel_id: Uuid::new_v4(),
    };
    let owner = agent(&scope, &dir.path().join("owner"), "apply").await;
    let mut other = agent(&sibling, &dir.path().join("sibling"), "apply").await;
    other.index = 1;
    let mut pool = AgentPool::from_slots(vec![Some(owner), Some(other)]);
    pool.queue_live_effort(&request(scope.channel_id(), "same-session", "high"), None);
    assert!(
        pool.try_claim(Some(&scope)).is_none(),
        "do not fall through and duplicate the held session on another worker"
    );
    assert_eq!(pool.try_claim(Some(&sibling)).unwrap().index, 1);
    pool.apply_pending_effort(None).await;
    assert_eq!(pool.try_claim(Some(&scope)).unwrap().index, 0);
    assert!(
        !dir.path().join("sibling").exists(),
        "never mutate the sibling adapter"
    );
}
