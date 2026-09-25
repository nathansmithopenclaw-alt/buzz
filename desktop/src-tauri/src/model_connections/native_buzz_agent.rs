//! Use buzz-agent's native Chat Completions configuration and session-only MCP.
use super::{qualification::Protocol, ModelConnection};
use std::process::Command;

pub(super) fn apply(
    command: &mut Command,
    connection: &ModelConnection,
    model: &str,
    key: Option<&str>,
) -> Result<(), String> {
    let qualified = connection
        .qualification
        .as_ref()
        .ok_or("Test this model connection in Settings before starting the agent")?;
    if !qualified
        .check(model, Protocol::ChatCompletions)
        .is_some_and(|check| check.tool_round_trip)
    {
        return Err("This connection has not passed Chat Completions tool qualification".into());
    }
    let context = qualified
        .context_window
        .filter(|size| *size >= 1024)
        .ok_or("Test this connection to establish its context window")?;
    // Native bounds, not the probe's small completion budget. These also bound
    // buzz-agent's existing context recovery/handoff behavior.
    let output = super::qualification::output_cap(context);
    command
        .env("BUZZ_AGENT_PROVIDER", "openai")
        .env("BUZZ_AGENT_MODEL", model)
        .env("OPENAI_COMPAT_MODEL", model)
        .env("OPENAI_COMPAT_BASE_URL", &connection.base_url)
        .env("OPENAI_COMPAT_API", "chat")
        .env("OPENAI_COMPAT_API_KEY", key.unwrap_or("local-no-auth"))
        .env("BUZZ_AGENT_MAX_CONTEXT_TOKENS", context.to_string())
        .env("BUZZ_AGENT_MAX_OUTPUT_TOKENS", output.to_string());
    // buzz-agent has no user/project server store. MCP tools come from
    // session/new; its MCP spawn clears the provider's inherited environment.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row() -> ModelConnection {
        serde_json::from_value(serde_json::json!({
            "id":"source", "name":"Fixture", "base_url":"http://localhost:8080/v1", "models":["raw/model"],
            "qualification":{"revision":1,"model":"raw/model","context_window":65536,"output_token_limit":8192,
            "protocols":[{"protocol":"chat_completions","tool_round_trip":true,"reason":null,"effort_levels":[]}]}
        })).unwrap()
    }
    #[test]
    fn selected_source_overrides_stale_provider_and_caps_without_key_arguments() {
        let mut command = Command::new("buzz-agent");
        command
            .env("BUZZ_AGENT_PROVIDER", "anthropic")
            .env("OPENAI_COMPAT_API", "responses")
            .env("BUZZ_AGENT_MODEL", "stale")
            .env("OPENAI_COMPAT_API_KEY", "stale");
        apply(
            &mut command,
            &row(),
            "raw/model",
            Some("synthetic-connection-key"),
        )
        .unwrap();
        let env: std::collections::BTreeMap<_, _> = command
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.unwrap().to_string_lossy().into_owned(),
                )
            })
            .collect();
        assert_eq!(env["BUZZ_AGENT_PROVIDER"], "openai");
        assert_eq!(env["BUZZ_AGENT_MODEL"], "raw/model");
        assert_eq!(env["OPENAI_COMPAT_BASE_URL"], "http://localhost:8080/v1");
        assert_eq!(env["OPENAI_COMPAT_API"], "chat");
        assert_eq!(env["OPENAI_COMPAT_API_KEY"], "synthetic-connection-key");
        assert_eq!(env["BUZZ_AGENT_MAX_CONTEXT_TOKENS"], "65536");
        assert_eq!(env["BUZZ_AGENT_MAX_OUTPUT_TOKENS"], "8192");
        assert_eq!(command.get_args().count(), 0);
    }
    #[test]
    fn unknown_context_or_unqualified_protocol_refuses_before_changing_command() {
        for context in [None, Some(0)] {
            let mut row = row();
            row.qualification.as_mut().unwrap().context_window = context;
            let mut command = Command::new("buzz-agent");
            assert!(apply(&mut command, &row, "raw/model", None).is_err());
            assert_eq!(command.get_envs().count(), 0);
        }
        let mut command = Command::new("buzz-agent");
        assert!(apply(&mut command, &row(), "other-model", None).is_err());
        apply(&mut command, &row(), "raw/model", None).unwrap();
    }
}
