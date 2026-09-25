use super::*;
use std::{
    io::{BufRead, Write},
    path::Path,
    process::{Command, Stdio},
};

#[test]
#[ignore = "requires installed Goose 1.45.0 and buzz-agent; bounded localhost only"]
fn installed_chat_projections_use_session_tools_and_native_output_caps() {
    for runtime in ["goose", "buzz-agent"] {
        let binary = std::env::var(if runtime == "goose" {
            "BUZZ_TEST_GOOSE_CLI"
        } else {
            "BUZZ_TEST_BUZZ_AGENT"
        })
        .expect("installed CLI required");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/chat_connection_recorder.py");
        let mut recorder = Command::new("python3")
            .arg(fixture)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = std::io::BufReader::new(recorder.stdout.take().unwrap());
        let mut first = String::new();
        output.read_line(&mut first).unwrap();
        let address: serde_json::Value = serde_json::from_str(&first).unwrap();
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let home = root.path().join("home");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&home).unwrap();
        let row: ModelConnection = serde_json::from_value(serde_json::json!({
            "id":"product-test", "name":"Product test", "base_url":address["base_url"], "models":["connection-test-model"],
            "qualification":{"revision":1,"model":"connection-test-model","context_window":65536,"output_token_limit":8192,
            "protocols":[{"protocol":"chat_completions","tool_round_trip":true,"reason":null,"effort_levels":[]}]}
        })).unwrap();
        let mut command = Command::new(&binary);
        command
            .env_clear()
            .current_dir(&workspace)
            .env("HOME", &home)
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("TMPDIR", root.path())
            .env("TERM", "dumb");
        let argv = if runtime == "goose" {
            native_goose::apply(
                &root.path().join("runtimes"),
                &mut command,
                &row,
                "connection-test-model",
                None,
                &"a".repeat(64),
                || Some(binary.clone().into()),
            )
            .unwrap();
            vec![
                binary.clone(),
                "acp".into(),
                "--with-builtin".into(),
                "developer".into(),
            ]
        } else {
            native_buzz_agent::apply(
                &mut command,
                &row,
                "connection-test-model",
                Some("synthetic-chat-product-test-key"),
            )
            .unwrap();
            vec![binary.clone()]
        };
        let env = command
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let spec = serde_json::json!({"runtime":runtime,"argv":argv,"cwd":workspace,"env":env});
        writeln!(recorder.stdin.as_mut().unwrap(), "{spec}").unwrap();
        recorder.stdin.take();
        let mut final_line = String::new();
        output.read_line(&mut final_line).unwrap();
        assert!(
            recorder.wait().unwrap().success(),
            "{runtime}: bounded native recorder failed"
        );
        let result: serde_json::Value = serde_json::from_str(&final_line).unwrap();
        assert_eq!(result["completed"], true, "{runtime}: {result}");
        assert_eq!(result["home_key_occurrences"], 0, "{runtime}");
        let requests = result["requests"].as_array().unwrap();
        assert_eq!(requests.len(), 2, "{runtime}: {result}");
        assert_eq!(requests[1]["mcp_env_clean"], true, "{runtime}: {result}");
        for request in requests {
            assert_eq!(request["model"], "connection-test-model", "{runtime}");
            assert_eq!(request["path"], "/v1/chat/completions", "{runtime}");
            assert_eq!(request["max_tokens"], 8192, "{runtime}");
            assert_eq!(request["key_in_tool_result"], 0, "{runtime}");
            assert_eq!(
                request["auth_matches"],
                runtime == "buzz-agent",
                "{runtime}"
            );
            let tools = request["tools"].as_array().unwrap();
            assert!(!tools
                .iter()
                .any(|tool| tool.as_str().unwrap().contains("account_fixture")));
            assert!(tools
                .iter()
                .any(|tool| tool.as_str().unwrap().ends_with("inspect_environment")));
        }
        println!("{runtime}: {result}");
    }
}
