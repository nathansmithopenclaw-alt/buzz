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
