use super::*;

#[test]
fn resolves_known_avatar_for_bare_command() {
    let avatar_url = managed_agent_avatar_url("goose").expect("goose avatar should resolve");

    assert_eq!(avatar_url, GOOSE_AVATAR_URL);
}

#[test]
fn resolves_known_avatar_for_command_paths_and_aliases() {
    assert_eq!(
        managed_agent_avatar_url("/usr/local/bin/codex-acp"),
        Some(CODEX_AVATAR_URL.to_string())
    );
    assert_eq!(
        managed_agent_avatar_url("Claude Code"),
        Some(CLAUDE_CODE_AVATAR_URL.to_string())
    );
    assert_eq!(
        managed_agent_avatar_url(r"C:\Tools\claude-agent-acp.exe"),
        Some(CLAUDE_CODE_AVATAR_URL.to_string())
    );
    assert_eq!(
        managed_agent_avatar_url("/usr/local/bin/claude-code-acp"),
        Some(CLAUDE_CODE_AVATAR_URL.to_string())
    );
}

#[test]
fn returns_none_for_unknown_commands() {
    assert!(managed_agent_avatar_url("custom-agent").is_none());
}

#[test]
fn recognizes_every_runtime_default_avatar() {
    for default_avatar in [
        GOOSE_AVATAR_URL,
        CLAUDE_CODE_AVATAR_URL,
        CODEX_AVATAR_URL,
        BUZZ_AGENT_AVATAR_URL,
    ] {
        assert!(
            is_managed_agent_default_avatar(default_avatar),
            "{default_avatar} is a runtime default"
        );
    }
}

#[test]
fn does_not_treat_a_chosen_picture_as_a_runtime_default() {
    assert!(!is_managed_agent_default_avatar(
        "https://relay.example/media/portrait.png"
    ));
    assert!(!is_managed_agent_default_avatar(""));
    // A near-miss must not match: comparison is exact, not a prefix.
    assert!(!is_managed_agent_default_avatar(&format!(
        "{GOOSE_AVATAR_URL}?v=2"
    )));
}

#[test]
fn default_agent_command_resolves_bundled_buzz_agent() {
    // The default must be bundled buzz-agent, never bare `goose` on a stock Windows install.
    assert_eq!(default_agent_command(), "buzz-agent");
    assert_eq!(
        normalize_agent_args(&default_agent_command(), vec!["acp".into()]),
        Vec::<String>::new()
    );
}

#[cfg(unix)]
#[test]
fn bundled_buzz_sidecar_wins_only_for_buzz_owned_binaries() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().expect("tempdir");
    let bundled = temp.path().join("buzz-acp");
    std::fs::write(&bundled, "#!/bin/sh\n").expect("write sidecar");
    std::fs::set_permissions(&bundled, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sidecar");

    assert_eq!(
        resolve_bundled_sidecar_from("buzz-acp", temp.path()).as_deref(),
        Some(bundled.as_path())
    );
    assert_eq!(
        resolve_bundled_sidecar_from("claude-agent-acp", temp.path()),
        None,
        "third-party adapters must keep managed/PATH resolution"
    );
    assert_eq!(
        resolve_bundled_sidecar_from("/custom/buzz-acp", temp.path()),
        None,
        "an explicit operator path must not be replaced"
    );
}
