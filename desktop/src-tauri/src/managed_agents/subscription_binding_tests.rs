use super::*;

#[test]
fn selected_codex_home_preserves_configuration_but_never_copies_authentication() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(source.join("config.toml"),"model = 'fixture-model'\napproval_policy = 'never'\nmodel_instructions_file = 'instructions.md'\n[mcp_servers.fixture]\ncommand = '/bin/true'\n").unwrap();
    std::fs::write(source.join("instructions.md"), "fixture instructions").unwrap();
    std::fs::write(source.join("AGENTS.md"), "fixture global instructions").unwrap();
    std::fs::write(
        source.join("auth.json"),
        "THIS IS A SYNTHETIC TEST SENTINEL",
    )
    .unwrap();
    let mut command = Command::new("/bin/sh");
    command
        .env_clear()
        .env("CODEX_HOME", &source)
        .env("OPENAI_API_KEY", "fixture-competing")
        .env("BUZZ_PRIVATE_KEY", "fixture-buzz")
        .env("BUZZ_ACP_MODEL", "selected-model");
    command.current_dir(root.path());
    apply_subscription_env(&mut command, "codex", &target).unwrap();
    command.args(["-c","printf '%s|%s|%s|%s' \"$CODEX_HOME\" \"${OPENAI_API_KEY-unset}\" \"$BUZZ_PRIVATE_KEY\" \"$BUZZ_ACP_MODEL\""]);
    let output = command.output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{}|unset|fixture-buzz|selected-model", target.display())
    );
    let config: toml::Table = std::fs::read_to_string(target.join("config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(config["model"].as_str(), Some("fixture-model"));
    assert_eq!(config["approval_policy"].as_str(), Some("never"));
    assert_eq!(
        config["model_instructions_file"].as_str(),
        source.join("instructions.md").to_str()
    );
    assert_eq!(config["cli_auth_credentials_store"].as_str(), Some("file"));
    assert_eq!(config["forced_login_method"].as_str(), Some("chatgpt"));
    assert_eq!(
        std::fs::read_to_string(target.join("AGENTS.md")).unwrap(),
        "fixture global instructions"
    );
    assert!(!target.join("auth.json").exists());
}

#[test]
fn selected_claude_account_preserves_config_model_and_communication_credentials() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("config");
    let target = root.path().join("account");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    let mut command = Command::new("/bin/sh");
    command
        .env_clear()
        .current_dir(root.path())
        .env("CLAUDE_CONFIG_DIR", &source)
        .env("ANTHROPIC_MODEL", "model-fixture")
        .env("CLAUDE_CODE_OAUTH_TOKEN", "legacy-fixture")
        .env("ANTHROPIC_API_KEY", "api-fixture")
        .env("BUZZ_AUTH_TAG", "buzz-fixture");
    apply_subscription_env(&mut command, "claude", &target).unwrap();
    command.args(["-c","printf '%s|%s|%s|%s|%s|%s' \"$CLAUDE_CONFIG_DIR\" \"$CLAUDE_SECURESTORAGE_CONFIG_DIR\" \"${CLAUDE_CODE_OAUTH_TOKEN-unset}\" \"${ANTHROPIC_API_KEY-unset}\" \"$ANTHROPIC_MODEL\" \"$BUZZ_AUTH_TAG\""]);
    let output = command.output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "{}|{}|unset|unset|model-fixture|buzz-fixture",
            source.display(),
            target.display()
        )
    );
}

#[test]
fn competing_configuration_authentication_fails_closed_without_target_write() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    for config in [
        "model_provider='other'",
        "[model_providers.openai]\nbase_url='https://example.invalid'",
        "[mcp_servers.test.env]\nOPENAI_API_KEY='fixture'",
    ] {
        std::fs::write(source.join("config.toml"), config).unwrap();
        let mut command = Command::new("/bin/true");
        assert!(compose_codex_config(&source, &target, &mut command).is_err());
        assert!(!target.join("config.toml").exists());
    }
    std::fs::write(
        source.join("settings.json"),
        r#"{"apiKeyHelper":"fixture-helper"}"#,
    )
    .unwrap();
    assert!(validate_claude_config(&source, None).is_err());
}

#[test]
fn project_auth_overrides_are_validated_from_the_child_working_directory() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let nested = project.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::create_dir_all(project.join(".codex")).unwrap();
    std::fs::create_dir_all(project.join(".claude")).unwrap();
    std::fs::write(project.join(".codex/config.toml"), "model_provider='other'").unwrap();
    assert!(validate_codex_project_config(Some(&nested)).is_err());
    std::fs::write(
        project.join(".claude/settings.local.json"),
        r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1"}}"#,
    )
    .unwrap();
    assert!(validate_claude_config(root.path(), Some(&nested)).is_err());
}

#[cfg(unix)]
#[test]
fn codex_config_link_cannot_redirect_runtime_writes_outside_owned_home() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    let unrelated = root.path().join("unrelated");
    std::fs::write(&unrelated, "untouched").unwrap();
    std::os::unix::fs::symlink(&unrelated, target.join("config.toml")).unwrap();
    assert!(compose_codex_config(&source, &target, &mut Command::new("/bin/true")).is_err());
    assert_eq!(std::fs::read_to_string(unrelated).unwrap(), "untouched");
}

#[test]
fn safe_switch_control_ignores_stale_ack_and_refuses_busy_before_selection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let token = "b".repeat(32);
    let worker_token = token.clone();
    std::fs::write(
        path.join("response.json"),
        serde_json::json!({"nonce":"prior-generation","token":token,"status":"quiesced"})
            .to_string(),
    )
    .unwrap();
    let responder = std::thread::spawn(move || {
        let start = Instant::now();
        while !path.join("request.json").exists() && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let request: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.join("request.json")).unwrap()).unwrap();
        assert_eq!(request["action"], "quiesce");
        assert_eq!(request["nonce"], "current-generation");
        super::super::atomic_write_json(
            &path.join("response.json"),
            serde_json::json!({"nonce":"current-generation","token":worker_token,"status":"busy"})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    });
    let result = control(dir.path(), "current-generation", &token, "quiesce");
    responder.join().unwrap();
    assert!(result.unwrap_err().contains("busy"));
}

#[test]
fn local_binding_survives_disk_roundtrip_but_never_enters_portable_definition() {
    let definition:super::super::AgentDefinition=serde_json::from_value(serde_json::json!({"id":"fixture","display_name":"Fixture","avatar_url":null,"system_prompt":"prompt","created_at":"now","updated_at":"now"})).unwrap();
    let mut record = definition.into_agent_record();
    record.subscription_account_id = Some("local-only-fixture".into());
    record.subscription_account_generation = 7;
    let bytes = serde_json::to_vec(&record).unwrap();
    let decoded: ManagedAgentRecord = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        decoded.subscription_account_id,
        record.subscription_account_id
    );
    assert_eq!(decoded.subscription_account_generation, 7);
    let portable = serde_json::to_string(&decoded.to_definition_view().unwrap()).unwrap();
    assert!(!portable.contains("local-only-fixture"));
    assert!(!portable.contains("subscription_account"));
}

#[cfg(unix)]
#[test]
fn control_response_never_follows_symlinks_or_reads_oversized_files() {
    let directory = tempfile::tempdir().unwrap();
    let unrelated = directory.path().join("unrelated-fixture");
    std::fs::write(&unrelated, "synthetic fixture content").unwrap();
    let response = directory.path().join("response.json");
    std::os::unix::fs::symlink(&unrelated, &response).unwrap();
    assert!(read_control_response(&response)
        .unwrap_err()
        .contains("response path"));
    std::fs::remove_file(&response).unwrap();
    std::fs::write(&response, vec![b'x'; 2049]).unwrap();
    assert!(read_control_response(&response)
        .unwrap_err()
        .contains("bounded"));
    std::fs::write(&response, "{}").unwrap();
    assert_eq!(
        read_control_response(&response).unwrap().as_deref(),
        Some("{}")
    );
}
