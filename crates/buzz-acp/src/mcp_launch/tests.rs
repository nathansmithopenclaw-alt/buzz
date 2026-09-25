use super::*;
use crate::acp::{AcpClient, EnvVar};

const KEY: &str = "nsec1-synthetic-argv-regression";
const AUTH: &str = "synthetic-owner-attestation-argv-regression";

fn server() -> McpServer {
    McpServer {
        name: "buzz-report".into(),
        command: "python3".into(),
        args: vec!["-c".into(), "import os,sys; assert os.environ['BUZZ_PRIVATE_KEY']==sys.argv[1]; assert os.environ['BUZZ_AUTH_TAG']==sys.argv[2]".into(), KEY.into(), AUTH.into()],
        env: vec![
            EnvVar { name: "BUZZ_PRIVATE_KEY".into(), value: KEY.into() },
            EnvVar { name: "BUZZ_AUTH_TAG".into(), value: AUTH.into() },
        ],
    }
}

// Deliberately hostile serialization: emulate an adapter putting the complete
// MCP declaration in a child argv. Bind the production new/load methods, not
// just the preparation helper. This is a fixture, not a vendor live-acceptance test.
const ADAPTER: &str = r#"
import json,subprocess,sys
for line in sys.stdin:
    req=json.loads(line)
    if req['method'] in ('session/new','session/load'):
        config=json.dumps(req['params']['mcpServers'])
        argv=json.loads(subprocess.check_output([sys.executable,'-c','import json,sys; print(json.dumps(sys.argv[1:]))','--mcp-config',config]))
        result={'sessionId':'fixture-session','argv':argv,'servers':req['params']['mcpServers']}
    else: result={}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
"#;

async fn check_runtime_argv(runtime: &str) {
    let mut client = AcpClient::spawn(
        "python3",
        &["-u".into(), "-c".into(), ADAPTER.into()],
        &[],
        false,
    )
    .await
    .unwrap();
    for load in [false, true] {
        let result = if load {
            client
                .session_load_full("fixture-session", "/tmp", vec![server()])
                .await
        } else {
            client
                .session_new_full("/tmp", vec![server()], None, None)
                .await
        }
        .unwrap();
        let argv = result.raw["argv"].to_string();
        assert!(
            !argv.contains(KEY) && !argv.contains(AUTH),
            "{runtime}: key material in spawned argv"
        );
        assert!(!argv.contains("BUZZ_PRIVATE_KEY") && !argv.contains("BUZZ_AUTH_TAG"));
        let path = Path::new(result.raw["servers"][0]["args"][1].as_str().unwrap());
        assert!(path.exists());
        // Repeated starts must retain the file and preserve explicit values,
        // even if the adapter clears inherited credentials before launching MCP.
        for _ in 0..2 {
            let mut child = command_from_file(path).unwrap();
            assert!(child.status().unwrap().success());
        }
    }
    client.shutdown().await;
}

macro_rules! runtime_tests {
    ($($name:ident),+ $(,)?) => {$ (
        #[tokio::test]
        async fn $name() { check_runtime_argv(stringify!($name)).await; }
    )+};
}

runtime_tests!(
    claude_argv_has_no_keys,
    codex_argv_has_no_keys,
    goose_argv_has_no_keys,
    buzz_agent_argv_has_no_keys,
    grok_argv_has_no_keys,
    pi_argv_has_no_keys,
    devin_argv_has_no_keys,
    cursor_argv_has_no_keys,
    omp_argv_has_no_keys,
    opencode_argv_has_no_keys,
    kimi_argv_has_no_keys,
    amp_argv_has_no_keys,
    hermes_argv_has_no_keys,
    openclaw_argv_has_no_keys,
    prime_argv_has_no_keys,
    deepseek_argv_has_no_keys,
    custom_argv_has_no_keys,
);

#[test]
fn private_files_preserve_config_and_are_removed_on_drop() {
    let (servers, files) = prepare(vec![server()]).unwrap();
    let path = Path::new(&servers[0].args[1]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    assert!(command_from_file(path).unwrap().status().unwrap().success());
    drop(files);
    assert!(!path.exists());
    assert!(command_from_file(path).is_err());
}

#[test]
fn ordinary_server_is_unchanged_and_invalid_file_fails_closed() {
    let mut ordinary = server();
    ordinary.env.clear();
    let original = serde_json::to_value(&ordinary).unwrap();
    let (servers, files) = prepare(vec![ordinary]).unwrap();
    assert_eq!(serde_json::to_value(&servers[0]).unwrap(), original);
    assert!(files.is_none());
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(b"not json synthetic-key").unwrap();
    assert_eq!(
        command_from_file(file.path()).unwrap_err().to_string(),
        "Invalid MCP launch configuration"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("link");
        symlink(file.path(), &link).unwrap();
        assert!(command_from_file(&link).is_err());
        std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(command_from_file(file.path()).is_err());
    }
}
