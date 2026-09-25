use super::*;
use std::io::{BufRead, Write};

#[test]
#[ignore = "requires installed qualified Codex CLI and adapter; localhost only"]
fn installed_native_projection_exec_keeps_the_whole_home_key_free() {
    let adapter = std::env::var("BUZZ_TEST_CODEX_ADAPTER").expect("adapter path required");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex_connection_recorder.py");
    let mut recorder = Command::new("python3")
        .arg(fixture)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = std::io::BufReader::new(recorder.stdout.take().unwrap());
    let mut first = String::new();
    output.read_line(&mut first).unwrap();
    let address: serde_json::Value = serde_json::from_str(&first).unwrap();
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(
        workspace.join("AGENTS.md"),
        "Use the supplied tools to complete the requested test.",
    )
    .unwrap();
    std::fs::create_dir(workspace.join(".codex")).unwrap();
    std::fs::write(workspace.join(".codex/config.toml"),
        "model_provider='redirect'\n[model_providers.redirect]\nname='Unselected fixture'\nbase_url='http://127.0.0.1:1/v1'\nwire_api='responses'\n[shell_environment_policy]\ninherit='all'\nignore_default_excludes=true\n").unwrap();
    let user_home = root.path().join("user-home");
    std::fs::create_dir(&user_home).unwrap();
    let row: ModelConnection = serde_json::from_value(serde_json::json!({
        "id":"product-test", "name":"Product test", "base_url":address["base_url"],
        "models":["connection-test-model"], "qualification":{
            "revision":1,"model":"connection-test-model","context_window":65536,
            "output_token_limit":8192,"protocols":[{"protocol":"responses","tool_round_trip":true,"reason":null,"effort_levels":[]}]
        }
    })).unwrap();
    let mut command = Command::new(&adapter);
    command
        .env_clear()
        .current_dir(&workspace)
        .env("HOME", &user_home)
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("BUZZ_ACP_AGENT_COMMAND", &adapter)
        .env("TMPDIR", root.path())
        .env("TERM", "dumb");
    let agent = "a".repeat(64);
    let runtimes = root.path().join("model-connection-runtimes");
    apply(
        &runtimes,
        &mut command,
        &row,
        "connection-test-model",
        Some("synthetic-codex-product-test-key"),
        &agent,
    )
    .unwrap();
    let home = command
        .get_envs()
        .find(|(key, _)| *key == "CODEX_HOME")
        .and_then(|(_, value)| value)
        .unwrap()
        .to_owned();
    let snapshot = Path::new(&home).join("shell_snapshots");
    std::fs::create_dir(&snapshot).unwrap();
    std::fs::write(snapshot.join("old.sh"), "synthetic-codex-product-test-key").unwrap();
    apply(
        &runtimes,
        &mut command,
        &row,
        "connection-test-model",
        Some("synthetic-codex-product-test-key"),
        &agent,
    )
    .unwrap();
    assert!(!snapshot.exists(), "every spawn removes old snapshots");
    let env = command
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.map(|v| v.to_string_lossy().into_owned()),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let spec = serde_json::json!({"adapter":adapter,"cwd":workspace,"env":env});
    writeln!(recorder.stdin.as_mut().unwrap(), "{spec}").unwrap();
    recorder.stdin.take();
    let mut final_line = String::new();
    output.read_line(&mut final_line).unwrap();
    assert!(
        recorder.wait().unwrap().success(),
        "bounded native recorder failed"
    );
    let result: serde_json::Value = serde_json::from_str(&final_line).unwrap();
    assert_eq!(result["completed"], true);
    assert_eq!(result["home_key_occurrences"], 0);
    assert_eq!(result["mode"], "agent");
    assert_eq!(result["metadata_warnings"], 0);
    let requests = result["requests"].as_array().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1]["exec_succeeded"], true);
    for request in requests {
        assert_eq!(request["model"], "connection-test-model");
        assert_eq!(request["auth_matches"], true);
        assert_eq!(request["key_in_result"], 0);
        assert_eq!(request["global_skill_markers"], 0);
        for tool in request["tools"].as_array().unwrap() {
            assert!(
                !["view_image", "web_search", "apply_patch", "tool_search"]
                    .contains(&tool["name"].as_str().unwrap_or("")),
                "unexpected tool: {tool}"
            );
            assert!(!["web_search", "web_search_preview"]
                .contains(&tool["type"].as_str().unwrap_or("")));
        }
    }
    assert!(!Path::new(&home).join("auth.json").exists());
    assert!(env.get("CODEX_PATH").and_then(Option::as_ref).is_none());
    println!("bundled_native_control={result}");
}

#[cfg(unix)]
#[test]
fn version_mismatch_refuses_before_creating_a_connection_home() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let adapter = root.path().join("codex-acp");
    std::fs::write(&adapter, "#!/bin/sh\necho 'codex-cli 0.154.0'\n").unwrap();
    std::fs::set_permissions(&adapter, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut command = Command::new("unused-adapter");
    command
        .current_dir(root.path())
        .env("BUZZ_ACP_AGENT_COMMAND", adapter);
    let row: ModelConnection = serde_json::from_value(serde_json::json!({
        "id":"test", "name":"Test", "base_url":"http://localhost/v1", "models":["model"],
        "qualification":{"revision":1,"model":"model","context_window":65536,"output_token_limit":8192,
        "protocols":[{"protocol":"responses","tool_round_trip":true,"reason":null,"effort_levels":[]}]}
    }))
    .unwrap();
    let runtimes = root.path().join("runtimes");
    let error = apply(
        &runtimes,
        &mut command,
        &row,
        "model",
        None,
        &"a".repeat(64),
    )
    .unwrap_err();
    assert!(error.contains("0.153.4"));
    assert!(error.contains("0.154.0"));
    assert!(!runtimes.exists());
}
