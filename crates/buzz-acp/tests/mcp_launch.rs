//! Exercise the packaged executable's stdio launcher with synthetic credentials.
#![cfg(unix)]

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::json;

#[test]
fn launcher_restores_child_env_without_secret_argv_and_preserves_stdio_cwd_exit() {
    let directory = tempfile::tempdir().unwrap();
    let mut file = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
    let key = "synthetic-private-key";
    let auth = "synthetic-auth-tag";
    // The program checks exact values without placing them in its own argv.
    // Hashes are fixture expectations, never hashes of a real credential.
    use sha2::{Digest, Sha256};
    let script = format!(
        "import hashlib,json,os,sys; assert hashlib.sha256(os.environ['BUZZ_PRIVATE_KEY'].encode()).hexdigest()=='{}'; assert hashlib.sha256(os.environ['BUZZ_AUTH_TAG'].encode()).hexdigest()=='{}'; assert os.getcwd()==os.path.realpath(sys.argv[1]); assert sys.stdin.readline()=='fixture input\\n'; print(json.dumps(sys.argv)); sys.exit(23)",
        hex::encode(Sha256::digest(key)), hex::encode(Sha256::digest(auth))
    );
    serde_json::to_writer(
        &mut file,
        &json!({
            "name": "fixture", "command": "python3",
            "args": ["-c", script, directory.path()],
            "env": [{"name":"BUZZ_PRIVATE_KEY","value":key}, {"name":"BUZZ_AUTH_TAG","value":auth}]
        }),
    )
    .unwrap();
    file.flush().unwrap();
    // Repeat starts prove the launch file is not consumed by the first child.
    for _ in 0..2 {
        let mut command = Command::new(env!("CARGO_BIN_EXE_buzz-acp"));
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .args(["mcp-launch"])
            .arg(file.path())
            .current_dir(directory.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let args = command
            .get_args()
            .map(|s| s.to_string_lossy())
            .collect::<String>();
        assert!(!args.contains(key) && !args.contains(auth));
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"fixture input\n")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(23),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let argv = String::from_utf8(output.stdout).unwrap();
        assert!(
            !argv.contains(key) && !argv.contains(auth),
            "final MCP argv must not contain credentials"
        );
        assert!(file.path().exists());
    }
    let path = file.path().to_owned();
    drop(file);
    assert!(!path.exists());
}

#[test]
fn launcher_missing_or_malformed_config_fails_without_echoing_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing");
    let missing = Command::new(env!("CARGO_BIN_EXE_buzz-acp"))
        .arg("mcp-launch")
        .arg(&path)
        .output()
        .unwrap();
    assert!(!missing.status.success());
    let mut file = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
    file.write_all(b"synthetic-secret-in-malformed-file")
        .unwrap();
    let invalid = Command::new(env!("CARGO_BIN_EXE_buzz-acp"))
        .arg("mcp-launch")
        .arg(file.path())
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(!String::from_utf8_lossy(&invalid.stderr).contains("synthetic-secret"));
}
