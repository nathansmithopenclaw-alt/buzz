//! Native projection of a saved connection into each supported harness.
use super::{resolve, source};
use std::{collections::BTreeMap, io::Write, path::Path, process::Command};
use tauri::{AppHandle, Manager};
pub(super) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}
fn prime_config(base: &str, model: &str) -> serde_json::Value {
    serde_json::json!({"providers":{"buzz-local":{
        "baseUrl":base,"api":"openai-completions","apiKey":"BUZZ_LOCAL_MODEL_API_KEY",
        "compat":{"supportsDeveloperRole":false,"supportsReasoningEffort":false},
        "models":[{"id":model,"name":model,"contextWindow":32768,"maxTokens":4096}]
    }}})
}
fn prime_args(existing: &str, model: &str) -> Result<String, String> {
    if model.contains(',') {
        return Err("Prime model names cannot contain commas".into());
    }
    // Prime project settings override its global settings.json. Explicit CLI
    // selection wins over both and errors instead of choosing a default model.
    Ok(format!("{existing},--provider,buzz-local,--model,{model}"))
}
/// Private launch data; deliberately neither Debug nor Serialize because it
/// owns the resolved credential. Hosted selections do not resolve a secret.
struct ConnectionLaunch {
    runtime: &'static str,
    connection: super::ModelConnection,
    key: Option<String>,
    model: String,
}

fn launch_plan(
    harness: &str,
    env: &BTreeMap<String, String>,
    model: Option<&str>,
    resolve: impl FnOnce(&str) -> Result<(super::ModelConnection, Option<String>), String>,
) -> Result<Option<ConnectionLaunch>, String> {
    let meta = crate::managed_agents::known_acp_runtime(harness);
    let provider = meta
        .and_then(|meta| meta.provider_env_var)
        .and_then(|key| env.get(key));
    let Some(id) = source::selected(env, provider.map(String::as_str))? else {
        return Ok(None);
    };
    let meta = meta.ok_or("This harness does not support saved model connections")?;
    if !matches!(meta.id, "deepseek" | "prime" | "codex") {
        return Err("This harness does not support saved model connections".into());
    }
    let (connection, key) = resolve(id)?;
    let model = source::model(&connection, model)?.to_owned();
    Ok(Some(ConnectionLaunch {
        runtime: meta.id,
        connection,
        key,
        model,
    }))
}

pub(crate) fn apply(
    app: &AppHandle,
    command: &mut Command,
    harness: &str,
    env: &BTreeMap<String, String>,
    model: Option<&str>,
    agent_key: &str,
) -> Result<(), String> {
    command.env_remove(buzz_harness_config::CODEX_SESSION_ONLY_ENV);
    let Some(plan) = launch_plan(harness, env, model, |id| resolve(app, id))? else {
        return Ok(());
    };
    let row = plan.connection;
    let key = plan.key;
    let model = plan.model.as_str();
    match plan.runtime {
        "codex" => {
            super::native_codex::apply(
                &app.path()
                    .app_data_dir()
                    .map_err(|e| e.to_string())?
                    .join("model-connection-runtimes"),
                command,
                &row,
                model,
                key.as_deref(),
                agent_key,
            )?;
        }
        "deepseek" => {
            command.env("DSH_PROVIDER", "deepseek-compatible");
            command.env("DSH_LLM_BASE_URL", &row.base_url);
            command.env("DSH_API_KEY", key.as_deref().unwrap_or("local-no-auth"));
            command.env("DSH_MODEL", model);
        }
        "prime" => {
            let existing_args = command
                .get_envs()
                .find(|(key, _)| *key == "BUZZ_ACP_AGENT_ARGS")
                .and_then(|(_, value)| value)
                .and_then(|value| value.to_str())
                .unwrap_or("--mode,acp");
            let args = prime_args(existing_args, model)?;
            command.env("BUZZ_ACP_AGENT_ARGS", args);
            if agent_key.len() != 64 || !agent_key.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err("Invalid agent identity for model connection".into());
            }
            let dir = app
                .path()
                .app_data_dir()
                .map_err(|e| e.to_string())?
                .join("model-connection-runtimes")
                .join(agent_key);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let bytes = serde_json::to_vec_pretty(&prime_config(&row.base_url, model))
                .map_err(|e| e.to_string())?;
            write_private(&dir.join("models.json"), &bytes)?;
            command.env("PRIME_AGENT_CODING_AGENT_DIR", dir);
            command.env(
                "BUZZ_LOCAL_MODEL_API_KEY",
                key.as_deref().unwrap_or("local-no-auth"),
            );
            let settings = serde_json::json!({"defaultProvider":"buzz-local","defaultModel":model});
            // Provider selection is explicit even when no ambient Prime account exists.
            let bytes = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
            let dir = app
                .path()
                .app_data_dir()
                .map_err(|e| e.to_string())?
                .join("model-connection-runtimes")
                .join(agent_key);
            write_private(&dir.join("settings.json"), &bytes)?;
        }
        _ => return Err("This harness does not support saved model connections".into()),
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launch_plan_requires_the_selected_source_without_a_provider_marker() {
        let env = BTreeMap::from([(super::super::CONNECTION_ENV.into(), "source-b".into())]);
        let plan = launch_plan("prime-agent", &env, Some("same-model"), |id| {
            assert_eq!(id, "source-b");
            Ok((
                super::super::ModelConnection {
                    id: id.into(),
                    name: "Second server".into(),
                    base_url: "http://127.0.0.1/v1".into(),
                    models: vec!["same-model".into()],
                    has_key: false,
                    secret_ref: None,
                    tested_model: None,
                    tested_at: None,
                    qualification: None,
                },
                None,
            ))
        })
        .unwrap()
        .unwrap();
        assert_eq!(plan.runtime, "prime");
        assert_eq!(plan.connection.id, "source-b");
        assert_eq!(plan.model, "same-model");
        assert!(
            launch_plan("prime-agent", &env, Some("same-model"), |_| Err(
                "removed".into()
            ))
            .is_err()
        );
    }

    #[test]
    fn hosted_and_unsupported_routes_do_not_read_connection_credentials() {
        fn no_credentials(
            _: &str,
        ) -> Result<(super::super::ModelConnection, Option<String>), String> {
            panic!("must not resolve connection credentials")
        }
        assert!(launch_plan(
            "claude-agent-acp",
            &BTreeMap::new(),
            Some("sonnet"),
            no_credentials
        )
        .unwrap()
        .is_none());
        let env = BTreeMap::from([(super::super::CONNECTION_ENV.into(), "source-a".into())]);
        assert!(launch_plan("claude-agent-acp", &env, Some("model"), no_credentials).is_err());
        assert!(launch_plan("unknown-adapter", &env, Some("model"), no_credentials).is_err());
    }

    #[test]
    fn prime_selection_is_explicit_even_with_inherited_provider_arguments() {
        let args =
            prime_args("--mode,acp,--provider,openrouter,--model,old", "Qwen/model").unwrap();
        assert!(args.ends_with(",--provider,buzz-local,--model,Qwen/model"));
        assert!(prime_args("--mode,acp", "bad,--provider,other").is_err());
    }
    #[test]
    fn connection_secrets_do_not_cross_public_serialization() {
        let row = super::super::ModelConnection {
            id: "fixture".into(),
            name: "Test".into(),
            base_url: "http://localhost/v1".into(),
            models: vec![],
            has_key: true,
            secret_ref: Some("private-reference".into()),
            tested_model: None,
            tested_at: None,
            qualification: None,
        };
        assert!(!serde_json::to_string(&row)
            .unwrap()
            .contains("private-reference"));
        let stored = super::super::StoredConnection {
            secret_ref: row.secret_ref.clone(),
            connection: row,
        };
        assert!(serde_json::to_string(&stored)
            .unwrap()
            .contains("private-reference"));
    }
    #[test]
    fn prime_config_uses_secret_reference_and_exact_endpoint_model() {
        let cfg = prime_config("http://localhost:8010/v1", "Qwen/model");
        assert_eq!(
            cfg["providers"]["buzz-local"]["models"][0]["id"],
            "Qwen/model"
        );
        assert_eq!(
            cfg["providers"]["buzz-local"]["apiKey"],
            "BUZZ_LOCAL_MODEL_API_KEY"
        );
        assert_eq!(
            cfg["providers"]["buzz-local"]["baseUrl"],
            "http://localhost:8010/v1"
        );
    }
}
