use super::model_switch_tests::switching_agent;
use super::tests::make_prompt_context_no_owner;
use super::*;

// Exercise the real task -> session/new -> model selection -> session/prompt
// seam. This peer would gladly execute a prompt on its default after either
// an unsupported selector or an explicit RPC rejection.
#[tokio::test]
async fn configured_model_gates_the_actual_prompt_wire() {
    for config_options in [false, true] {
        for (catalog, desired, reject, allowed, expected_status) in [
            (
                true,
                Some("claude-fable-5[1m]"),
                false,
                false,
                Some("unsupported_model"),
            ),
            (true, Some("default[]"), true, false, Some("failure")),
            (true, Some("default[]"), false, true, None),
            (true, None, false, true, None),
            (false, Some("spawn-selected"), false, true, None),
        ] {
            let script = format!(
                r#"import json,sys
catalog={catalog}
reject={reject}
config_options={config_options}
for line in sys.stdin:
 r=json.loads(line); method=r['method']
 result={{}}
 if method=='session/new':
  result={{'sessionId':'fixture-session'}}
  if catalog and config_options: result['configOptions']=[{{'configId':'model','category':'model','currentValue':'default[]','options':[{{'value':'default[]','name':'Auto'}}]}}]
  if catalog and not config_options: result['models']={{'currentModelId':'default[]','availableModels':[{{'modelId':'default[]','name':'Auto'}}]}}
 elif method=='session/prompt': result={{'stopReason':'end_turn'}}
 reply={{'jsonrpc':'2.0','id':r['id'],'result':result}}
 if method in ['session/set_model','session/set_config_option'] and reject:
  reply={{'jsonrpc':'2.0','id':r['id'],'error':{{'code':-32602,'message':'model rejected'}}}}
 print(json.dumps(reply),flush=True)
"#,
                config_options = if config_options { "True" } else { "False" },
                catalog = if catalog { "True" } else { "False" },
                reject = if reject { "True" } else { "False" }
            );
            let acp = AcpClient::spawn("python3", &["-u".into(), "-c".into(), script], &[], false)
                .await
                .unwrap();
            let mut agent = switching_agent(acp, desired.unwrap_or("unused"));
            agent.desired_model = desired.map(str::to_owned);
            agent.model_overridden = false;
            let obs = observer::ObserverHandle::in_process();
            agent.acp.set_observer(Some(obs.clone()), 0);
            let (tx, mut rx) = mpsc::unbounded_channel();
            run_prompt_task(
                agent,
                None,
                Some("perform one task".into()),
                Arc::new(make_prompt_context_no_owner()),
                tx,
                None,
                "configured-model-test".into(),
            )
            .await;
            let mut result = rx.recv().await.unwrap();
            let events = obs.snapshot();
            let prompts = events
                .iter()
                .filter(|e| e.kind == "acp_write" && e.payload["method"] == "session/prompt")
                .count();
            assert_eq!(
                prompts,
                usize::from(allowed),
                "desired={desired:?}, catalog={catalog}, reject={reject}"
            );
            assert_eq!(matches!(result.outcome, PromptOutcome::Error(_)), !allowed);
            if !allowed {
                assert!(result.agent.state.heartbeat_session.is_none());
                assert_eq!(result.agent.state.heartbeat_turn_count, 0);
                assert!(!events.iter().any(|e| e.kind == "session_config_captured"));
            }
            if let Some(status) = expected_status {
                assert!(events
                    .iter()
                    .any(|e| e.kind == "control_result" && e.payload["status"] == status));
            }
            result.agent.acp.shutdown().await;
        }
    }
}
