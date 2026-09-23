use super::*;

use std::ffi::OsStr;

const CLAUDE_ADAPTER: &str = "/Users/example/.nvm/versions/node/v22/bin/claude-agent-acp";

fn command_for(
    agent_command: &str,
    model: Option<&str>,
    env: &[(&str, &str)],
) -> std::process::Command {
    let merged_env = env
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect::<BTreeMap<_, _>>();
    agent_models_command(
        std::path::Path::new("buzz-acp"),
        agent_command,
        &[],
        model,
        &merged_env,
    )
    .expect("discovery command builds")
}

fn args(command: &std::process::Command) -> Vec<String> {
    command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

/// The command's explicit write for `key`: `Some(Some(v))` set, `Some(None)`
/// removed, `None` untouched (inherits the Desktop process env).
fn env_write(command: &std::process::Command, key: &str) -> Option<Option<String>> {
    command
        .get_envs()
        .find(|(name, _)| *name == OsStr::new(key))
        .map(|(_, value)| value.map(|v| v.to_string_lossy().into_owned()))
}

#[test]
fn claude_discovery_selects_the_saved_model_the_way_spawn_does() {
    let command = command_for(CLAUDE_ADAPTER, Some("claude-opus-5-5"), &[]);

    assert_eq!(args(&command), ["models", "--json"]);
    assert_eq!(
        env_write(&command, "ANTHROPIC_MODEL"),
        Some(Some("claude-opus-5-5".to_string()))
    );
    assert_eq!(env_write(&command, "BUZZ_ACP_MODEL"), Some(None));
}

#[test]
fn claude_discovery_saved_model_wins_over_hand_set_model_env() {
    let command = command_for(
        CLAUDE_ADAPTER,
        Some(" claude-fable-5-1[1m] "),
        &[
            ("ANTHROPIC_MODEL", "claude-sonnet-5"),
            ("BUZZ_ACP_MODEL", "claude-sonnet-5"),
        ],
    );

    assert_eq!(args(&command), ["models", "--json"]);
    assert_eq!(
        env_write(&command, "ANTHROPIC_MODEL"),
        Some(Some("claude-fable-5-1[1m]".to_string()))
    );
    assert_eq!(env_write(&command, "BUZZ_ACP_MODEL"), Some(None));
}

#[test]
fn claude_discovery_without_a_saved_model_probes_the_harness_default() {
    for model in [None, Some(""), Some("   ")] {
        let command = command_for(
            "claude-agent-acp",
            model,
            &[("ANTHROPIC_MODEL", "claude-sonnet-5")],
        );

        assert_eq!(args(&command), ["models", "--json"], "model {model:?}");
        assert_eq!(
            env_write(&command, "ANTHROPIC_MODEL"),
            Some(None),
            "model {model:?}"
        );
    }
}

#[test]
fn claude_discovery_recognizes_the_resolved_windows_adapter_shim() {
    // Callers pass the resolved adapter path; on Windows that is an npm shim.
    for command in [
        r"C:\Users\example\AppData\Roaming\npm\claude-agent-acp.cmd",
        r"C:\Users\example\AppData\Roaming\npm\CLAUDE-AGENT-ACP.CMD",
        "claude-agent-acp.bat",
    ] {
        let discovery = command_for(command, Some("claude-opus-5-5"), &[]);

        assert_eq!(args(&discovery), ["models", "--json"], "{command}");
        assert_eq!(
            env_write(&discovery, "ANTHROPIC_MODEL"),
            Some(Some("claude-opus-5-5".to_string())),
            "{command}"
        );
    }

    let codex = command_for(r"C:\npm\codex-acp.cmd", Some("gpt-5.6-sol"), &[]);
    assert_eq!(args(&codex), ["models", "--json", "--model", "gpt-5.6-sol"]);
}

#[test]
fn switch_runtimes_keep_the_explicit_model_argument() {
    let command = command_for("codex-acp", Some("gpt-5.6-sol[medium]"), &[]);

    assert_eq!(
        args(&command),
        ["models", "--json", "--model", "gpt-5.6-sol[medium]"]
    );
    assert_eq!(env_write(&command, "ANTHROPIC_MODEL"), None);
}

fn discovery_output(current_model: &str) -> serde_json::Value {
    serde_json::json!({
        "agent": { "name": "claude-agent-acp", "version": "0.54.1" },
        "stable": {
            "configOptions": [
                {
                    "id": "model",
                    "category": "model",
                    "currentValue": current_model,
                    "options": [{ "value": current_model }]
                },
                {
                    "id": "effort",
                    "category": "thought_level",
                    "options": [{ "value": "default" }, { "value": "max" }]
                }
            ]
        },
        "unstable": { "currentModelId": current_model, "availableModels": [] }
    })
}

#[test]
fn startup_selected_model_is_not_reported_as_the_harness_default() {
    let response = agent_models_response(
        &discovery_output("claude-opus-5-5"),
        Some("claude-opus-5-5".to_string()),
        CLAUDE_ADAPTER,
    );

    assert_eq!(response.agent_default_model, None);
    assert_eq!(response.selected_model.as_deref(), Some("claude-opus-5-5"));
    assert_eq!(response.effort_config_id.as_deref(), Some("effort"));
    assert_eq!(response.effort_options.len(), 2);
}

#[test]
fn harness_default_survives_when_no_model_was_selected_at_startup() {
    let unset = agent_models_response(&discovery_output("default"), None, CLAUDE_ADAPTER);
    assert_eq!(unset.agent_default_model.as_deref(), Some("default"));

    let switched = agent_models_response(
        &discovery_output("gpt-5.6-sol"),
        Some("gpt-5.6-sol".to_string()),
        "codex-acp",
    );
    assert_eq!(switched.agent_default_model.as_deref(), Some("gpt-5.6-sol"));
}
