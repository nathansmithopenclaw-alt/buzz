//! Goose 1.45 native projection: keyless, explicit session tools, private state.
use super::{qualification::Protocol, ModelConnection};
use std::{path::Path, process::Command};

pub(super) const KEYED_REASON: &str = "Goose 1.45 exposes the provider key to its shell, so pick Codex or buzz-agent for this connection.";
pub(super) use buzz_harness_config::goose::{validate_version, SESSION_ONLY_ENV};

pub(super) fn validate_key(keyed: bool) -> Result<(), String> {
    if keyed {
        Err(KEYED_REASON.into())
    } else {
        Ok(())
    }
}
fn probe(cli: &Path, root: &Path) -> Result<(), String> {
    let mut command = Command::new(cli);
    command
        .arg("--version")
        .env("GOOSE_PATH_ROOT", root)
        .env("GOOSE_DISABLE_KEYRING", "1");
    validate_version(buzz_harness_config::cli_version::probe_command(
        &mut command,
    ))
}
pub(super) fn runtime_reason() -> Option<String> {
    let Some(cli) = crate::managed_agents::resolve_command("goose") else {
        return Some("Install Goose 1.45.0 to use this connection".into());
    };
    let root = match tempfile::tempdir() {
        Ok(root) => root,
        Err(_) => return Some("Cannot inspect the installed Goose version".into()),
    };
    probe(&cli, root.path()).err()
}

pub(super) fn apply(
    root: &Path,
    command: &mut Command,
    connection: &ModelConnection,
    model: &str,
    key: Option<&str>,
    agent: &str,
    resolve_cli: impl FnOnce() -> Option<std::path::PathBuf>,
) -> Result<(), String> {
    validate_key(connection.has_key || key.is_some())?;
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
    let cli = resolve_cli().ok_or("Install Goose 1.45.0 to use this connection")?;
    let home = super::runtime_home::prepare_for(root, agent, connection, model, "goose")?;
    probe(&cli, &home)?;
    let url = url::Url::parse(&connection.base_url).map_err(|_| "Invalid connection URL")?;
    let path = format!("{}/chat/completions", url.path().trim_matches('/'))
        .trim_start_matches('/')
        .to_owned();
    command
        .env("BUZZ_ACP_AGENT_COMMAND", &cli)
        .env("BUZZ_ACP_AGENT_ARGS", "acp,--with-builtin,developer")
        .env(SESSION_ONLY_ENV, "1")
        .env("GOOSE_PATH_ROOT", home)
        .env("GOOSE_DISABLE_KEYRING", "1")
        .env("GOOSE_ADDITIONAL_CONFIG_FILES", "")
        .env("EXTENSIONS", "{}")
        .env("GOOSE_PROVIDER", "openai")
        .env("GOOSE_MODEL", model)
        .env("OPENAI_HOST", url.origin().ascii_serialization())
        .env("OPENAI_BASE_PATH", path)
        .env("OPENAI_API_KEY", "local-no-auth")
        .env("OPENAI_CUSTOM_HEADERS", "")
        .env("GOOSE_PROVIDER__API_KEY", "local-no-auth")
        .env("GOOSE_PROVIDER__HOST", &connection.base_url)
        .env("OPENAI_COMPAT_API_KEY", "local-no-auth")
        .env("OPENAI_COMPAT_BASE_URL", &connection.base_url)
        .env("GOOSE_CONTEXT_LIMIT", context.to_string())
        .env(
            "GOOSE_MAX_TOKENS",
            super::qualification::output_cap(context).to_string(),
        )
        .env("GOOSE_DISABLE_SESSION_NAMING", "true");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyed_connections_refuse_and_keyless_connections_pass() {
        assert_eq!(validate_key(true).unwrap_err(), KEYED_REASON);
        validate_key(false).unwrap();
    }
    #[test]
    fn only_qualified_goose_version_passes() {
        validate_version(Some((1, 45, 0))).unwrap();
        let error = validate_version(Some((1, 52, 0))).unwrap_err();
        assert!(error.contains("1.45.0") && error.contains("1.52.0"));
    }
    #[cfg(unix)]
    #[test]
    fn spawn_refuses_keys_and_unknown_versions_but_projects_keyless_connections() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let cli = root.path().join("goose");
        std::fs::write(&cli, "#!/bin/sh\necho 'goose 1.45.0'\n").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut row: ModelConnection = serde_json::from_value(serde_json::json!({
            "id":"source", "name":"Source", "base_url":"http://localhost:8000/v1", "models":["raw/model"],
            "qualification":{"revision":1,"model":"raw/model","context_window":65536,"output_token_limit":8192,
            "protocols":[{"protocol":"chat_completions","tool_round_trip":true,"reason":null,"effort_levels":[]}]}
        })).unwrap();
        let mut command = Command::new("broker");
        let homes = root.path().join("runtimes");
        let agent = "a".repeat(64);
        for (has_key, resolved) in [(true, None), (false, Some("synthetic-key"))] {
            row.has_key = has_key;
            assert_eq!(
                apply(
                    &homes,
                    &mut command,
                    &row,
                    "raw/model",
                    resolved,
                    &agent,
                    || Some(cli.clone())
                )
                .unwrap_err(),
                KEYED_REASON
            );
            assert_eq!(command.get_envs().count(), 0);
            assert!(!homes.exists());
        }
        row.has_key = false;
        apply(
            &homes,
            &mut command,
            &row,
            "raw/model",
            None,
            &agent,
            || Some(cli.clone()),
        )
        .unwrap();
        let env: std::collections::BTreeMap<_, _> = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.unwrap().to_string_lossy().into_owned(),
                )
            })
            .collect();
        assert_eq!(env["GOOSE_MODEL"], "raw/model");
        assert_eq!(env["OPENAI_HOST"], "http://localhost:8000");
        assert_eq!(env["OPENAI_BASE_PATH"], "v1/chat/completions");
        assert_eq!(env["GOOSE_MAX_TOKENS"], "8192");
        assert_eq!(env["GOOSE_CONTEXT_LIMIT"], "65536");
        assert_eq!(env["OPENAI_API_KEY"], "local-no-auth");
        assert_eq!(env[SESSION_ONLY_ENV], "1");
        assert_eq!(env["EXTENSIONS"], "{}");
        assert!(Path::new(&env["GOOSE_PATH_ROOT"]).starts_with(&homes));
        assert_eq!(command.get_args().count(), 0);
        std::fs::write(&cli, "#!/bin/sh\necho 'goose 1.52.0'\n").unwrap();
        let mut command = Command::new("broker");
        let error = apply(
            &homes,
            &mut command,
            &row,
            "raw/model",
            None,
            &agent,
            || Some(cli.clone()),
        )
        .unwrap_err();
        assert!(error.contains("1.45.0") && error.contains("1.52.0"));
        assert_eq!(command.get_envs().count(), 0);
    }
}
