use crate::acp::AcpClient;
use std::os::unix::fs::PermissionsExt;

/// Re-exec the real test binary to exercise Desktop's inherited binding without
/// mutating environment variables in the parallel parent test runner.
#[tokio::test]
async fn token_binding_enforces_spawn_boundary() {
    const MARKER: &str = "BUZZ_TEST_TOKEN_CHILD";
    const TEST: &str = "acp::subscription_token::tests::token_binding_enforces_spawn_boundary";
    if std::env::var_os(MARKER).is_none() {
        for scenario in [
            "unreadable",
            "valid",
            "late",
            "missing",
            "public",
            "symlink",
            "directory",
            "oversized",
            "invalid",
            "unknown",
            "codex",
        ] {
            let root = tempfile::tempdir().unwrap();
            let adapter = root.path().join(if scenario == "codex" {
                "codex-acp"
            } else {
                "claude-agent-acp"
            });
            std::fs::write(&adapter, r#"#!/usr/bin/python3
import json, os, sys
for line in sys.stdin:
    req = json.loads(line)
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':{
      'selected':os.environ.get('CLAUDE_CODE_OAUTH_TOKEN') == 'sk-ant-oat01-' + 'f'*95,
      'securestorage':'CLAUDE_SECURESTORAGE_CONFIG_DIR' in os.environ,
      'override':any(k in os.environ for k in ['ANTHROPIC_API_KEY','CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR']),
      'config':os.environ.get('CLAUDE_CONFIG_DIR') == os.environ.get('BUZZ_ACP_SUBSCRIPTION_CONFIG_HOME')}}), flush=True)
"#).unwrap();
            std::fs::set_permissions(&adapter, std::fs::Permissions::from_mode(0o700)).unwrap();
            let token = root.path().join("oauth-token");
            std::fs::write(&token, format!("sk-ant-oat01-{}", "f".repeat(95))).unwrap();
            std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o600)).unwrap();
            match scenario {
                "missing" => std::fs::remove_file(&token).unwrap(),
                "unreadable" => {
                    std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o000))
                        .unwrap()
                }
                "public" => {
                    std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o644))
                        .unwrap()
                }
                "symlink" => {
                    let target = root.path().join("target");
                    std::fs::rename(&token, &target).unwrap();
                    std::os::unix::fs::symlink(target, &token).unwrap();
                }
                "directory" => {
                    std::fs::remove_file(&token).unwrap();
                    std::fs::create_dir(&token).unwrap();
                }
                "oversized" => std::fs::write(&token, "f".repeat(4097)).unwrap(),
                "invalid" => std::fs::write(&token, "f".repeat(108)).unwrap(),
                _ => {}
            }
            let mut test = std::process::Command::new(std::env::current_exe().unwrap());
            test.args(["--exact", TEST, "--nocapture"])
                .env(MARKER, &adapter)
                .env("BUZZ_TEST_TOKEN_SCENARIO", scenario)
                .env("BUZZ_ACP_SUBSCRIPTION_ACCOUNT_ID", "fixture")
                .env(
                    "BUZZ_ACP_SUBSCRIPTION_KIND",
                    if scenario == "unknown" {
                        "future"
                    } else {
                        "token"
                    },
                )
                .env(
                    "BUZZ_ACP_SUBSCRIPTION_HARNESS",
                    if scenario == "codex" {
                        "codex"
                    } else {
                        "claude"
                    },
                )
                .env("BUZZ_ACP_SUBSCRIPTION_HOME", root.path())
                .env(
                    "BUZZ_ACP_SUBSCRIPTION_CONFIG_HOME",
                    root.path().join("agent-config"),
                );
            for key in [
                "CLAUDE_CODE_OAUTH_TOKEN",
                "ANTHROPIC_API_KEY",
                "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR",
                "CLAUDE_SECURESTORAGE_CONFIG_DIR",
            ] {
                if scenario == "late" {
                    test.env_remove(key);
                } else {
                    test.env(key, "fixture-caller-override");
                }
            }
            let output = test.output().unwrap();
            assert!(
                output.status.success(),
                "{scenario}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return;
    }
    let adapter = std::env::var(MARKER).unwrap();
    let scenario = std::env::var("BUZZ_TEST_TOKEN_SCENARIO").unwrap();
    let overrides = [
        "CLAUDE_CODE_OAUTH_TOKEN",
        "ANTHROPIC_API_KEY",
        "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR",
        "CLAUDE_SECURESTORAGE_CONFIG_DIR",
        "CLAUDE_CONFIG_DIR",
    ]
    .into_iter()
    .map(|key| (key.into(), "fixture-late-override".into()))
    .collect::<Vec<_>>();
    let mut clients = vec![];
    for _ in 0..5 {
        let result = AcpClient::spawn(&adapter, &[], &overrides, false).await;
        if scenario != "valid" && scenario != "late" {
            let error = match result {
                Err(error) => error.to_string(),
                Ok(mut client) => {
                    client.shutdown().await;
                    panic!("unsafe binding reached a provider child: {scenario}");
                }
            };
            assert!(
                error.contains(if scenario == "unknown" || scenario == "codex" {
                    "Invalid subscription account kind"
                } else {
                    "Selected subscription token is missing, unreadable, or unsafe"
                }),
                "{error}"
            );
            assert!(!error.contains("sk-ant-oat01-"));
            return;
        }
        let mut client = result.unwrap();
        let observed = client
            .send_request("fixture", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(
            observed,
            serde_json::json!({"selected":true,"securestorage":false,"override":false,"config":true})
        );
        clients.push(client);
    }
    for mut client in clients {
        client.shutdown().await;
    }
}
