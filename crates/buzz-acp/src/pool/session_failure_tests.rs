use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// The scripted adapter behaves like AIR adapters: terminal failure metadata
// appears only if the actual initialize request opts in. Removing that opt-in
// must turn negotiated_terminal_failure_reaches_delivery_notice red.
async fn scripted_adapter(mode: &str) -> AcpClient {
    let script = r#"
import json, sys
mode = sys.argv[1]
negotiated = False
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    method = req['method']
    air = {'version': 1, 'capabilities': ['sessionFailure']}
    if method == 'initialize':
        caps = req['params']['clientCapabilities'].get('_meta', {}).get('jetbrains', {}).get('air', {})
        negotiated = caps.get('version') == 1 and 'sessionFailure' in caps.get('capabilities', [])
        result = {'protocolVersion': 1, 'agentCapabilities': {}, '_meta': {'jetbrains': {'air': air}}}
        if mode == 'unadvertised': result.pop('_meta')
    elif method == 'session/new': result = {'sessionId': 'test-session'}
    elif method == 'session/prompt':
        failure = {'id': 'turn:error', 'revision': 1, 'category': 'request', 'severity': 'error', 'title': 'SECRET_PROVIDER_BODY', 'actions': []}
        # Stale/retry notifications alone must never classify the current turn.
        print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'test-session','update':{'sessionUpdate':'session_info_update','_meta':{'jetbrains':{'air':{'version':1,'sessionFailure':failure}}}}}}), flush=True)
        print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'test-session','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'ordinary terminal answer'}}}}), flush=True)
        result = {'stopReason':'end_turn'}
        if negotiated and mode != 'notification-only':
            if mode in ('warning','info'): failure['severity'] = mode
            if mode == 'malformed': failure['severity'] = 42
            if mode == 'cleared': failure = None
            result['_meta'] = {'jetbrains':{'air':{'version':1,'sessionFailure':failure}}}
    else: result = {}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}), flush=True)
"#;
    let mut client = AcpClient::spawn(
        "python3",
        &["-u".into(), "-c".into(), script.into(), mode.into()],
        &[],
        false,
    )
    .await
    .unwrap();
    client.initialize().await.unwrap();
    client
}

struct RelayCapture {
    url: String,
    events: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for RelayCapture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn capture_relay() -> RelayCapture {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = events.clone();
    let server = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut bytes = Vec::new();
            let (body_start, body_len, is_event) = loop {
                let mut chunk = [0; 8192];
                let count = socket.read(&mut chunk).await.unwrap();
                assert!(count > 0 && bytes.len() < 1024 * 1024);
                bytes.extend_from_slice(&chunk[..count]);
                let mut headers = [httparse::EMPTY_HEADER; 64];
                let mut request = httparse::Request::new(&mut headers);
                if let httparse::Status::Complete(start) = request.parse(&bytes).unwrap() {
                    let len = request
                        .headers
                        .iter()
                        .find(|h| h.name.eq_ignore_ascii_case("content-length"))
                        .map(|h| {
                            std::str::from_utf8(h.value)
                                .unwrap()
                                .parse::<usize>()
                                .unwrap()
                        })
                        .unwrap_or(0);
                    break (start, len, request.path == Some("/events"));
                }
            };
            while bytes.len() < body_start + body_len {
                let mut chunk = [0; 8192];
                let count = socket.read(&mut chunk).await.unwrap();
                assert!(count > 0 && bytes.len() < 1024 * 1024);
                bytes.extend_from_slice(&chunk[..count]);
            }
            let response = if is_event {
                let event: serde_json::Value =
                    serde_json::from_slice(&bytes[body_start..body_start + body_len]).unwrap();
                captured.lock().unwrap().push(event);
                "{\"accepted\":true}"
            } else {
                "[]"
            };
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
        }
    });
    RelayCapture {
        url,
        events,
        server,
    }
}

async fn broker_turn(
    mut acp: AcpClient,
    is_dm: bool,
    cwd: &str,
    model: Option<&str>,
) -> (PromptResult, Vec<serde_json::Value>, SessionScope) {
    let relay = capture_relay().await;
    let channel = Uuid::new_v4();
    let scope = conv(channel);
    let keys = Keys::generate();
    let session = acp.session_new_full(cwd, vec![], None, None).await.unwrap();
    if let Some(model) = model {
        acp.session_set_config_option(&session.session_id, "model", model)
            .await
            .unwrap();
    }
    let mut agent = OwnedAgent {
        index: 0,
        acp,
        state: SessionState::default(),
        model_capabilities: None,
        desired_model: None,
        model_overridden: false,
        desired_model_request_id: None,
        desired_model_pending_ack: false,
        startup_effort: None,
        agent_name: "test-adapter".into(),
        goose_system_prompt_supported: None,
        protocol_version: 1,
    };
    agent
        .state
        .sessions
        .insert(scope.clone(), session.session_id);
    let mut ctx = make_prompt_context_impl(&keys, None);
    ctx.cwd = cwd.into();
    ctx.system_prompt = Some("You are Qwenproof in an isolated broker acceptance probe. Reply exactly FAILURE_SIGNAL_CONTROL_OK. Do not use tools or change files.".into());
    ctx.rest_client.base_url = relay.url.clone();
    ctx.channel_info = ChannelInfoResolver::new(
        HashMap::from([(
            channel,
            crate::relay::ChannelInfo {
                name: "Qwenproof failure signal probe".into(),
                channel_type: if is_dm { "dm" } else { "stream" }.into(),
                description: None,
            },
        )]),
        ctx.rest_client.clone(),
    );
    let event = EventBuilder::new(
        Kind::Custom(9),
        "Reply exactly FAILURE_SIGNAL_CONTROL_OK. Do not use tools.",
    )
    .tags([
        Tag::parse(["h", &channel.to_string()]).unwrap(),
        Tag::public_key(keys.public_key()),
    ])
    .sign_with_keys(&Keys::generate())
    .unwrap();
    let batch = batch_with_scope(scope.clone(), event);
    let (tx, mut rx) = mpsc::unbounded_channel();
    tokio::time::timeout(
        Duration::from_secs(150),
        run_prompt_task(
            agent,
            Some(batch),
            None,
            Arc::new(ctx),
            tx,
            None,
            Uuid::new_v4().to_string(),
        ),
    )
    .await
    .unwrap();
    let mut result = rx.recv().await.unwrap();
    result.agent.acp.shutdown().await;
    // Include the production publication settle window and a second window to
    // catch duplicate notices. The capture returns verified-empty relay pulls.
    tokio::time::sleep(PUBLICATION_SETTLE_DELAY + Duration::from_millis(350)).await;
    let events = relay.events.lock().unwrap().clone();
    (result, events, scope)
}

fn notices(events: &[serde_json::Value]) -> Vec<&serde_json::Value> {
    events
        .iter()
        .filter(|event| {
            event["tags"].as_array().is_some_and(|tags| {
                tags.iter()
                    .any(|tag| *tag == json!(["buzz-acp", "delivery-failure"]))
            })
        })
        .collect()
}

#[tokio::test]
async fn negotiated_terminal_failure_reaches_delivery_notice() {
    for is_dm in [false, true] {
        let (result, events, scope) =
            broker_turn(scripted_adapter("error").await, is_dm, ".", None).await;
        assert!(matches!(
            result.outcome,
            PromptOutcome::Error(AcpError::AgentRuntimeFailure)
        ));
        assert!(!result.agent.state.sessions.contains_key(&scope));
        assert!(
            result
                .agent
                .acp
                .turn_delivery_evidence()
                .adapter_runtime_failure_present
        );
        let notices = notices(&events);
        assert_eq!(notices.len(), 1, "{events:?}");
        assert!(notices[0]["content"]
            .as_str()
            .unwrap()
            .contains("agent_runtime_failed"));
        assert!(!notices[0].to_string().contains("SECRET_PROVIDER_BODY"));
    }
}

#[tokio::test]
async fn nonterminal_and_unadvertised_metadata_preserve_success() {
    for mode in [
        "warning",
        "info",
        "cleared",
        "unadvertised",
        "notification-only",
    ] {
        let (result, events, scope) =
            broker_turn(scripted_adapter(mode).await, false, ".", None).await;
        assert!(
            matches!(result.outcome, PromptOutcome::Ok(StopReason::EndTurn)),
            "{mode}"
        );
        assert!(result.agent.state.sessions.contains_key(&scope));
        assert!(
            !result
                .agent
                .acp
                .turn_delivery_evidence()
                .adapter_runtime_failure_present
        );
        assert!(notices(&events).is_empty(), "{mode}: {events:?}");
    }
}

#[tokio::test]
async fn malformed_negotiated_failure_is_a_protocol_error() {
    let (result, _, scope) =
        broker_turn(scripted_adapter("malformed").await, false, ".", None).await;
    assert!(matches!(
        result.outcome,
        PromptOutcome::Error(AcpError::Protocol(_))
    ));
    assert!(!result.agent.state.sessions.contains_key(&scope));
}

// Compiles the real broker prompt-task, session invalidation, and signed notice
// path against an installed adapter; only the relay is an isolated capture.
#[tokio::test]
#[ignore = "requires installed authenticated adapter; set BUZZ_TEST_FAILURE_ADAPTER and BUZZ_TEST_FAILURE_CASE"]
async fn installed_adapter_through_broker_failure_signal() {
    assert!(
        std::env::var_os("CODEX_CONFIG").is_none(),
        "scrub inherited CODEX_CONFIG so operator-wins precedence cannot replace the probe model"
    );
    let command = std::env::var("BUZZ_TEST_FAILURE_ADAPTER").unwrap();
    let case = std::env::var("BUZZ_TEST_FAILURE_CASE").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let unknown = "keel-deliberately-unknown-model-no-inference-20260925";
    let extra_env = if case == "codex-unknown" {
        vec![("CODEX_CONFIG".into(), json!({"model":unknown,"features.apps":false,"features.plugins":false,"features.remote_plugin":false}).to_string())]
    } else if case == "claude-unknown" {
        vec![("ANTHROPIC_CUSTOM_MODEL_OPTION".into(), unknown.into())]
    } else {
        vec![]
    };
    let mut acp = AcpClient::spawn(&command, &[], &extra_env, false)
        .await
        .unwrap();
    let init = acp.initialize().await.unwrap();
    assert_eq!(
        init.pointer("/_meta/jetbrains/air/version"),
        Some(&json!(1))
    );
    let model = (case == "claude-unknown").then_some(unknown);
    let (result, events, scope) =
        broker_turn(acp, false, dir.path().to_str().unwrap(), model).await;
    let failed = case != "claude-success";
    assert_eq!(
        result
            .agent
            .acp
            .turn_delivery_evidence()
            .adapter_runtime_failure_present,
        failed
    );
    assert_eq!(result.agent.state.sessions.contains_key(&scope), !failed);
    assert_eq!(notices(&events).len(), usize::from(failed), "{events:?}");
    if failed {
        assert!(matches!(
            result.outcome,
            PromptOutcome::Error(AcpError::AgentRuntimeFailure)
        ));
        assert!(notices(&events)[0]["content"]
            .as_str()
            .unwrap()
            .contains("agent_runtime_failed"));
    } else {
        assert!(matches!(
            result.outcome,
            PromptOutcome::Ok(StopReason::EndTurn)
        ));
        assert!(result
            .agent
            .acp
            .turn_delivery_evidence()
            .final_answer_snippet
            .as_deref()
            .unwrap_or_default()
            .split_whitespace()
            .collect::<String>()
            .contains("FAILURE_SIGNAL_CONTROL_OK"));
    }
    println!("case={case} typed_failure={failed} session_invalidated={failed} delivery_failure_notices={}", notices(&events).len());
}
