use super::*;
use crate::{acp::EnvVar, execution_evidence as evidence};

fn fixture() -> (tempfile::TempDir, PromptContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = make_prompt_context_no_owner();
    ctx.mcp_servers = vec![McpServer {
        name: "buzz-computer-use".into(),
        command: "computer-use".into(),
        args: vec![],
        env: vec![EnvVar {
            name: "BUZZ_COMPUTER_REGISTRY".into(),
            value: dir.path().canonicalize().unwrap().to_string_lossy().into(),
        }],
    }];
    (dir, ctx)
}

fn grant(ctx: &PromptContext, scope: &SessionScope) -> (std::path::PathBuf, String) {
    let servers = evidence::session_servers(ctx, Some(scope), ctx.mcp_servers.clone()).unwrap();
    let computer = servers
        .iter()
        .find(|s| s.name == "buzz-computer-use")
        .unwrap();
    assert!(!computer.env.iter().any(|e| e.name == "BUZZ_PRIVATE_KEY"));
    let report = servers.iter().find(|s| s.name == "buzz-report").unwrap();
    assert_eq!(report.args, vec!["report-mcp"]);
    let get = |name| {
        computer
            .env
            .iter()
            .find(|e| e.name == name)
            .unwrap()
            .value
            .clone()
    };
    (
        get("BUZZ_EXECUTION_ROOT").into(),
        get("BUZZ_EXECUTION_SESSION"),
    )
}

#[test]
fn cancelled_and_replayed_turns_retain_unknown_effects_and_exact_reply_destination() {
    let (_dir, ctx) = fixture();
    let scope = conv(Uuid::new_v4());
    let (root, session) = grant(&ctx, &scope);
    let batch = batch_with_scope(scope.clone(), signed_event_with_tags(vec![]));
    let first = evidence::begin(&ctx, Some(&batch), &Uuid::new_v4().to_string())
        .unwrap()
        .unwrap();
    evidence::write(
        &root
            .join("turns")
            .join(&first.turn.turn_id)
            .join("calls/one.json"),
        &json!({"tool":"browser_action","mutation":true,"state":"started"}),
        true,
    )
    .unwrap();
    first.revoke().unwrap();
    assert!(evidence::active(&root, &session).is_err());
    let mut next = batch_with_scope(scope, signed_event_with_tags(vec![]));
    next.cancelled_events = batch.events.clone();
    let second = evidence::begin(&ctx, Some(&next), &Uuid::new_v4().to_string())
        .unwrap()
        .unwrap();
    assert!(second.turn.recovery);
    assert_eq!(second.turn.reply_event_id, next.events[0].event.id.to_hex());
    assert_eq!(second.turn.root_event_id, next.events[0].event.id.to_hex());
    assert_eq!(
        evidence::summary(&root, &second.turn).unwrap()["unknown_mutation_outcomes"],
        1
    );
    assert!(
        evidence::prompt(&second)
            .unwrap()
            .contains("browser_action")
    );
    drop(first); // An old guard cannot retire a newer turn.
    assert!(evidence::active(&root, &session).is_ok());
    drop(second);
    // Process restart/replay may reset attempt to one; event lineage still fences input.
    let replay = evidence::begin(&ctx, Some(&batch), &Uuid::new_v4().to_string())
        .unwrap()
        .unwrap();
    assert!(replay.turn.recovery);
}

#[test]
fn relay_agent_channel_and_thread_scopes_are_isolated_and_session_grants_revoke() {
    let (_dir, mut ctx) = fixture();
    let channel_id = Uuid::new_v4();
    let scope = conv(channel_id);
    let (root, session) = grant(&ctx, &scope);
    let batch = batch_with_scope(scope.clone(), signed_event_with_tags(vec![]));
    let active = evidence::begin(&ctx, Some(&batch), &Uuid::new_v4().to_string())
        .unwrap()
        .unwrap();
    let (same, replacement) = grant(&ctx, &scope);
    assert_eq!(same, root);
    assert_ne!(replacement, session);
    assert!(evidence::active(&root, &session).is_err());
    assert_ne!(grant(&ctx, &conv(Uuid::new_v4())).0, root);
    assert_ne!(
        grant(
            &ctx,
            &SessionScope::Thread {
                channel_id,
                root_event_id: "ab".repeat(32)
            }
        )
        .0,
        root
    );
    ctx.rest_client.base_url = "http://other-community.invalid".into();
    let other_relay = grant(&ctx, &scope).0;
    assert_ne!(other_relay, root);
    ctx.rest_client.keys = Keys::generate();
    assert_ne!(grant(&ctx, &scope).0, other_relay);
    drop(active);
}

#[test]
fn incomplete_or_symlinked_evidence_fails_closed() {
    let (_dir, ctx) = fixture();
    let scope = conv(Uuid::new_v4());
    let (root, _) = grant(&ctx, &scope);
    let batch = batch_with_scope(scope, signed_event_with_tags(vec![]));
    let active = evidence::begin(&ctx, Some(&batch), &Uuid::new_v4().to_string())
        .unwrap()
        .unwrap();
    let calls = root.join("turns").join(&active.turn.turn_id).join("calls");
    std::fs::write(calls.join("broken.json"), "{").unwrap();
    assert!(evidence::summary(&root, &active.turn).is_err());
    #[cfg(unix)]
    {
        std::fs::remove_file(calls.join("broken.json")).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("fact.json"), "{}").unwrap();
        std::os::unix::fs::symlink(outside.path(), calls.join("link")).unwrap();
        assert!(evidence::read::<serde_json::Value>(&calls.join("link/fact.json")).is_err());
    }
}

async fn completion_fixture(
    mode: &str,
    harness: &str,
) -> (
    tempfile::TempDir,
    PromptContext,
    evidence::ActiveTurn,
    crate::acp::AcpClient,
) {
    let (dir, mut ctx) = fixture();
    ctx.harness_name = harness.into();
    ctx.max_turn_duration = Duration::from_secs(10);
    let scope = conv(Uuid::new_v4());
    grant(&ctx, &scope);
    let batch = batch_with_scope(scope, signed_event_with_tags(vec![]));
    let active = evidence::begin(&ctx, Some(&batch), &Uuid::new_v4().to_string())
        .unwrap()
        .unwrap();
    let thread = buzz_sdk::ThreadRef {
        root_event_id: nostr::EventId::from_hex(&active.turn.root_event_id).unwrap(),
        parent_event_id: nostr::EventId::from_hex(&active.turn.reply_event_id).unwrap(),
    };
    let event = buzz_sdk::build_message(
        active.turn.channel_id,
        "literal report",
        Some(&thread),
        &[],
        false,
        &[],
        &[],
    )
    .unwrap()
    .sign_with_keys(&ctx.rest_client.keys)
    .unwrap();
    evidence::write(
        &active.root.join("fixture-report.json"),
        &json!({"content_hash":"fixture", "event":event}),
        true,
    )
    .unwrap();
    evidence::write(
        &active.root.join("fixture-receipt.json"),
        &json!({"event_id":event.id,"readback_verified":true}),
        true,
    )
    .unwrap();
    let script = dir.path().join("provider.py");
    std::fs::write(&script, r#"
import sys,json,pathlib,shutil
root=pathlib.Path(sys.argv[1]);mode=sys.argv[2];count=0;last=None
for line in sys.stdin:
 r=json.loads(line)
 if r.get('method')=='session/prompt':
  count+=1;last=r['id'];active=json.loads((root/'active.json').read_text())
  with (root/'prompts.jsonl').open('a') as f:f.write(json.dumps({'count':count,'report_only':active.get('report_only',False)})+'\n')
  if count==2 and mode=='hang':continue
  if count==2 and mode=='publish':
   target=root/'turns'/active['turn_id']
   shutil.copyfile(root/'fixture-report.json',target/'report.json')
   shutil.copyfile(root/'fixture-receipt.json',target/'report-receipt.json')
  text='initial unpublished output' if count==1 else 'completion output'
  print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'session','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':text}}}}),flush=True)
  print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':{'stopReason':'end_turn'}}),flush=True)
 elif r.get('method')=='session/cancel':
  print(json.dumps({'jsonrpc':'2.0','id':last,'result':{'stopReason':'cancelled'}}),flush=True)
"#).unwrap();
    let client = crate::acp::AcpClient::spawn(
        "python3",
        &[
            script.to_string_lossy().into(),
            active.root.to_string_lossy().into(),
            mode.into(),
        ],
        &[],
        false,
    )
    .await
    .unwrap();
    (dir, ctx, active, client)
}

fn completion_calls(active: &evidence::ActiveTurn) -> Vec<serde_json::Value> {
    std::fs::read_to_string(active.root.join("prompts.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn completion_publishes_once_with_input_fenced_and_delivery_evidence_preserved() {
    let (_dir, ctx, active, mut client) = completion_fixture("publish", "opencode").await;
    assert_eq!(
        crate::report_completion::prompt(&mut client, "session", &["work"], Some(&active), &ctx)
            .await
            .unwrap(),
        StopReason::EndTurn
    );
    let calls = completion_calls(&active);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["report_only"], false);
    assert_eq!(calls[1]["report_only"], true);
    let delivery = client.turn_delivery_evidence();
    let snippet = delivery.final_answer_snippet.unwrap();
    assert!(
        snippet.contains("initial unpublished output") && snippet.contains("completion output")
    );
    assert!(
        active
            .root
            .join("turns")
            .join(&active.turn.turn_id)
            .join("report-receipt.json")
            .exists()
    );
    client.shutdown_for_account_switch().await.unwrap();
}

#[tokio::test]
async fn completion_is_bounded_and_missing_receipt_still_requires_a_loud_failure() {
    let (_dir, ctx, active, mut client) = completion_fixture("ignore", "opencode").await;
    crate::report_completion::prompt(&mut client, "session", &["work"], Some(&active), &ctx)
        .await
        .unwrap();
    assert_eq!(completion_calls(&active).len(), 2);
    assert!(matches!(
        publication_silence_disposition(
            "turn",
            &client.turn_delivery_evidence(),
            true,
            Some(&StopReason::EndTurn)
        ),
        PublicationSilenceDisposition::LoudFailure(_)
    ));
    assert!(
        !active
            .root
            .join("turns")
            .join(&active.turn.turn_id)
            .join("report.json")
            .exists()
    );
    client.shutdown_for_account_switch().await.unwrap();
}

#[tokio::test]
async fn completion_preserves_existing_delivery_other_adapters_and_turn_budget() {
    for mode in ["already_published", "other_adapter", "short_budget"] {
        let (_dir, mut ctx, active, mut client) = completion_fixture(
            "ignore",
            if mode == "other_adapter" {
                "goose"
            } else {
                "opencode"
            },
        )
        .await;
        if mode == "already_published" {
            let target = active.root.join("turns").join(&active.turn.turn_id);
            std::fs::copy(
                active.root.join("fixture-report.json"),
                target.join("report.json"),
            )
            .unwrap();
            std::fs::copy(
                active.root.join("fixture-receipt.json"),
                target.join("report-receipt.json"),
            )
            .unwrap();
        }
        if mode == "short_budget" {
            ctx.max_turn_duration = Duration::from_secs(4);
        }
        crate::report_completion::prompt(&mut client, "session", &["work"], Some(&active), &ctx)
            .await
            .unwrap();
        assert_eq!(completion_calls(&active).len(), 1, "{mode}");
        assert!(
            !evidence::active(&active.root, &active.turn.session)
                .unwrap()
                .report_only
        );
        client.shutdown_for_account_switch().await.unwrap();
    }
}

#[tokio::test]
async fn cancellation_during_report_completion_retains_the_input_fence_and_uses_existing_cancel() {
    let (_dir, ctx, active, mut client) = completion_fixture("hang", "opencode").await;
    tokio::select! {
        _ = crate::report_completion::prompt(&mut client, "session", &["work"], Some(&active), &ctx) => panic!("completion should remain pending"),
        _ = async {
            loop {
                if std::fs::read_to_string(active.root.join("prompts.jsonl")).is_ok_and(|s| s.lines().count()==2) { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        } => {}
    }
    active.revoke().unwrap();
    assert!(client.has_in_flight_prompt());
    assert_eq!(
        client
            .cancel_with_cleanup_grace("session", Duration::from_secs(1))
            .await
            .unwrap(),
        StopReason::Cancelled
    );
    assert!(evidence::active(&active.root, &active.turn.session).is_err());
    assert_eq!(completion_calls(&active).len(), 2);
    client.shutdown_for_account_switch().await.unwrap();
}
