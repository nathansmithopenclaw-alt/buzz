//! Native projection of a saved connection into each supported harness.
use super::{resolve, CONNECTION_ENV};
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
pub(crate) fn apply(
    app: &AppHandle,
    command: &mut Command,
    harness: &str,
    env: &BTreeMap<String, String>,
    model: Option<&str>,
    agent_key: &str,
) -> Result<(), String> {
    let Some(meta) = crate::managed_agents::known_acp_runtime(harness) else {
        return Ok(());
    };
    let provider = meta.provider_env_var.and_then(|key| env.get(key));
    if provider.map(String::as_str) != Some("buzz-local") {
        return Ok(());
    }
    let id = env
        .get(CONNECTION_ENV)
        .filter(|s| !s.is_empty())
        .ok_or("Choose a saved model connection")?;
    let (row, key) = resolve(app, id)?;
    let model = model
        .filter(|s| !s.is_empty())
        .ok_or("Choose a model from the saved connection")?;
    if model.len() > 256 || model.chars().any(char::is_control) {
        return Err("Invalid model name".into());
    }
    match meta.id {
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
