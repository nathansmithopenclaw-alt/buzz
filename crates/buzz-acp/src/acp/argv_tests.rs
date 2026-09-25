//! Production-bound transport tests. The fixture deliberately serializes the
//! complete ACP MCP declaration into a subprocess argv, the worst-case adapter
//! behavior. This is distinct from acceptance against installed vendor binaries.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use crate::acp::{AcpClient, EnvVar, McpServer};

const KEY: &str = "nsec1-synthetic-argv-regression";
const AUTH: &str = "synthetic-owner-attestation-argv-regression";
const ADAPTER: &str = r#"#!/usr/bin/env python3
import json,subprocess,sys
for line in sys.stdin:
    req=json.loads(line)
    if req['method']=='session/load' and '--exit-on-load' in sys.argv: break
    if req['method'] in ('session/new','session/load'):
        config=json.dumps(req['params']['mcpServers'])
        argv=json.loads(subprocess.check_output([sys.executable,'-c','import json,sys; print(json.dumps(sys.argv[1:]))','--mcp-config',config]))
        result={'sessionId':'fixture-session','argv':argv,'servers':req['params']['mcpServers']}
    elif req['method']=='fixture/exit': break
    elif req['method']=='session/prompt': result={'stopReason':'end_turn'}
    else: result={}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
"#;

fn servers() -> Vec<McpServer> {
    ["buzz-dev-mcp", "buzz-report"]
        .into_iter()
        .map(|name| McpServer {
            name: name.into(),
            command: "fixture-mcp".into(),
            args: vec!["argument with spaces".into()],
            env: vec![
                EnvVar {
                    name: "BUZZ_PRIVATE_KEY".into(),
                    value: KEY.into(),
                },
                EnvVar {
                    name: "BUZZ_AUTH_TAG".into(),
                    value: AUTH.into(),
                },
            ],
        })
        .collect()
}

async fn client(command: &str, args: &[&str]) -> (tempfile::TempDir, AcpClient) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(command);
    std::fs::write(&path, ADAPTER).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let client = AcpClient::spawn(
        path.to_str().unwrap(),
        &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        &[],
        false,
    )
    .await
    .unwrap();
    (dir, client)
}

fn assert_argv_and_files(result: &serde_json::Value, runtime: &str) -> Vec<PathBuf> {
    let argv = result["argv"].to_string();
    assert!(
        !argv.contains(KEY) && !argv.contains(AUTH),
        "{runtime}: key material in spawned argv"
    );
    assert!(!argv.contains("BUZZ_PRIVATE_KEY") && !argv.contains("BUZZ_AUTH_TAG"));
    result["servers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|server| {
            assert_eq!(server["args"][0], "mcp-launch");
            assert_eq!(server["env"], serde_json::json!([]));
            let path = PathBuf::from(server["args"][1].as_str().unwrap());
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            // Read twice: credentials must remain available for adapter MCP retries.
            for _ in 0..2 {
                let original: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                assert_eq!(original["env"][0]["value"], KEY);
                assert_eq!(original["env"][1]["value"], AUTH);
                assert_eq!(original["name"], server["name"]);
                assert_eq!(original["args"][0], "argument with spaces");
            }
            path
        })
        .collect()
}

async fn check_runtime(command: &str, args: &[&str]) {
    let (_dir, mut client) = client(command, args).await;
    let new = client
        .session_new_full("/tmp", servers(), None, None)
        .await
        .unwrap();
    let old = assert_argv_and_files(&new.raw, command);
    let loaded = client
        .session_load_full("fixture-session", "/tmp", servers())
        .await
        .unwrap();
    let current = assert_argv_and_files(&loaded.raw, command);
    assert!(
        old.iter().all(|p| !p.exists()),
        "superseded session files must be removed"
    );
    client.shutdown().await;
    assert!(
        current.iter().all(|p| !p.exists()),
        "shutdown must remove session files"
    );
}

macro_rules! runtime_tests {
    ($($name:ident => ($command:literal, [$($arg:literal),*])),+ $(,)?) => {$ (
        #[tokio::test]
        async fn $name() { check_runtime($command, &[$($arg),*]).await; }
    )+};
}

runtime_tests!(
    claude_argv_has_no_keys => ("claude-agent-acp", []),
    claude_legacy_argv_has_no_keys => ("claude-code-acp", []),
    codex_argv_has_no_keys => ("codex-acp", []),
    goose_argv_has_no_keys => ("goose", ["acp"]),
    buzz_agent_argv_has_no_keys => ("buzz-agent", []),
    grok_argv_has_no_keys => ("grok", ["agent", "--always-approve", "stdio"]),
    pi_argv_has_no_keys => ("buzz-pi-acp", []),
    devin_argv_has_no_keys => ("devin", ["acp"]),
    cursor_argv_has_no_keys => ("cursor-agent", ["acp"]),
    omp_argv_has_no_keys => ("omp", ["acp"]),
    opencode_argv_has_no_keys => ("opencode", ["acp"]),
    kimi_argv_has_no_keys => ("kimi", ["acp"]),
    amp_argv_has_no_keys => ("amp-acp", []),
    hermes_argv_has_no_keys => ("hermes-acp", []),
    openclaw_argv_has_no_keys => ("openclaw", ["acp"]),
    prime_argv_has_no_keys => ("prime-agent", ["--mode", "acp"]),
    deepseek_argv_has_no_keys => ("dsh", ["--profile", "buzz-acp"]),
    custom_argv_has_no_keys => ("custom-adapter", []),
);

#[tokio::test]
async fn drop_and_session_close_remove_private_files() {
    for acknowledged in [false, true] {
        let (_dir, mut client) = client("fixture", &[]).await;
        let new = client
            .session_new_full("/tmp", servers(), None, None)
            .await
            .unwrap();
        let paths = assert_argv_and_files(&new.raw, "close");
        if acknowledged {
            client
                .session_close_acknowledged("fixture-session")
                .await
                .unwrap();
        } else {
            client
                .session_close_fire_and_forget("fixture-session")
                .await
                .unwrap();
        }
        assert!(paths.iter().all(|p| !p.exists()));
    }
    let (_dir, mut client) = client("fixture", &[]).await;
    let new = client
        .session_new_full("/tmp", servers(), None, None)
        .await
        .unwrap();
    let paths = assert_argv_and_files(&new.raw, "drop");
    drop(client);
    assert!(paths.iter().all(|p| !p.exists()));
}

#[tokio::test]
async fn adapter_exit_and_account_switch_remove_private_files() {
    let (_dir, mut client) = client("fixture", &["--exit-on-load"]).await;
    let new = client
        .session_new_full("/tmp", servers(), None, None)
        .await
        .unwrap();
    let paths = assert_argv_and_files(&new.raw, "exit");
    assert!(client
        .session_load_full("fixture-session", "/tmp", servers())
        .await
        .is_err());
    assert!(paths.iter().all(|p| !p.exists()));
    drop(client);

    let (_dir, mut client) = self::client("fixture", &[]).await;
    let new = client
        .session_new_full("/tmp", servers(), None, None)
        .await
        .unwrap();
    let paths = assert_argv_and_files(&new.raw, "switch");
    client.shutdown_for_account_switch().await.unwrap();
    assert!(paths.iter().all(|p| !p.exists()));
}
