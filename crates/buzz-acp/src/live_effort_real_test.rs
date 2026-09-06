//! Opt-in provider acceptance: two real turns around the production effort queue.
//! Requires BUZZ_TEST_CODEX_ADAPTER and an already-authenticated CODEX_HOME.
use super::*;
use crate::acp::AcpClient;
use crate::pool::{OwnedAgent, SessionState, TaskMeta};
use crate::scope::SessionScope;
use std::collections::HashMap;

#[tokio::test]
#[ignore = "requires authenticated Codex; performs two real provider turns"]
async fn real_codex_preserves_conversation_across_queued_effort_change() {
    let command = std::env::var("BUZZ_TEST_CODEX_ADAPTER").expect("set BUZZ_TEST_CODEX_ADAPTER");
    let cwd = tempfile::tempdir().unwrap();
    let mut acp = AcpClient::spawn(&command, &[], &[], false).await.unwrap();
    let init = acp.initialize().await.unwrap();
    let session = acp
        .session_new_full(
            cwd.path().to_str().unwrap(),
            vec![],
            None,
            Some("Buzz live effort acceptance"),
        )
        .await
        .unwrap();
    let session_id = session.session_id;
    let model =
        std::env::var("BUZZ_TEST_CODEX_MODEL").unwrap_or_else(|_| "gpt-5.6-sol[low]".into());
    let switched = acp.session_set_model(&session_id, &model).await.unwrap();
    let config = if switched.get("configOptions").is_some() {
        switched
    } else {
        session.raw
    };
    let option = effort_option(&config).expect("native thought_level option");
    assert!(supports_value(&option["options"], "high"));
    let low = acp
        .session_set_config_option(&session_id, option["id"].as_str().unwrap(), "low")
        .await
        .unwrap();
    assert_eq!(effort_option(&low).unwrap()["currentValue"], "low");
    let scope = SessionScope::Conversation {
        channel_id: Uuid::new_v4(),
    };
    let mut state = SessionState::default();
    state.sessions.insert(scope.clone(), session_id.clone());
    state.configs.insert(scope.clone(), low);
    let mut worker = OwnedAgent {
        index: 0,
        acp,
        state,
        model_capabilities: None,
        desired_model: None,
        model_overridden: false,
        desired_model_request_id: None,
        desired_model_pending_ack: false,
        startup_effort: Some("low".into()),
        agent_name: "codex".into(),
        goose_system_prompt_supported: None,
        protocol_version: 2,
    };
    let observer = ObserverHandle::in_process();
    let mut pool = AgentPool::from_slots(vec![None]);
    let nonce = Uuid::new_v4().to_string();
    let prompt = format!("Do not use any tools. Remember this verification word for the next turn: {nonce}. Reply with exactly REMEMBERED.");
    let first_session = session_id.clone();
    let running = tokio::spawn(async move {
        let response = worker
            .acp
            .session_prompt_capture_text(
                &first_session,
                &prompt,
                Duration::from_secs(90),
                Duration::from_secs(180),
            )
            .await
            .unwrap();
        (worker, response)
    });
    let task_id = running.id();
    let (control_tx, mut control_rx) = tokio::sync::oneshot::channel();
    pool.task_map_mut().insert(
        task_id,
        TaskMeta {
            agent_index: 0,
            channel_id: Some(scope.channel_id()),
            scope: Some(scope.clone()),
            turn_id: "real-provider-turn".into(),
            recoverable_batch: None,
            control_tx: Some(control_tx),
            steer_tx: None,
            successful_steer_deliveries: HashMap::new(),
        },
    );
    let request = json!({"requestId":Uuid::new_v4(),"channelId":scope.channel_id(),"sessionId":session_id,"effort":"high"});
    pool.queue_live_effort(&request, Some(&observer));
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(
        observer.snapshot().last().unwrap().payload["status"],
        "queued"
    );
    assert!(matches!(
        control_rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    let (worker, (_, first_text)) = running.await.unwrap();
    assert!(
        first_text.contains("REMEMBERED"),
        "unexpected first response: {first_text}"
    );
    pool.task_map_mut().remove(&task_id);
    pool.return_agent(worker);
    assert!(
        pool.try_claim(Some(&scope)).is_none(),
        "pending edit fences next dispatch"
    );
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(
        observer.snapshot().last().unwrap().payload["status"],
        "applied"
    );
    let mut worker = pool.try_claim(Some(&scope)).unwrap();
    assert_eq!(worker.state.sessions[&scope], session_id);
    assert_eq!(
        effort_option(&worker.state.configs[&scope]).unwrap()["currentValue"],
        "high"
    );
    assert_eq!(worker.startup_effort.as_deref(), Some("low"));
    let (_, second_text) = worker.acp.session_prompt_capture_text(&session_id, "Do not use tools. Reply with only the verification word I gave you in the previous turn.", Duration::from_secs(90), Duration::from_secs(180)).await.unwrap();
    assert!(
        second_text.contains(&nonce),
        "conversation memory lost: {second_text}"
    );
    worker.acp.shutdown().await;
    println!("REAL_CODEX_EFFORT_OK adapter={} model={} session={} low->high; busy queued; unchanged session; memory retained; startup default low", init["agentInfo"], model, session_id);
}
