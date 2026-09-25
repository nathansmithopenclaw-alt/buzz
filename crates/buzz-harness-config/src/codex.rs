//! Codex 0.153.4 native metadata and config for an explicitly selected endpoint.
//! The API key is referenced by env name, never accepted by these functions.
use serde_json::{json, Value};
use std::path::Path;

pub const PROVIDER_KEY_ENV: &str = "BUZZ_LOCAL_MODEL_API_KEY";

/// Changes to this list require rerunning native key-path and namespace controls.
pub const QUALIFIED_VERSIONS: &[(u64, u64, u64)] = &[(0, 153, 4)];

/// Probe the adapter's bundled CLI through its own Node launcher. `cli -V`
/// forwards to codex.js; `--version` would report codex-acp's version instead.
pub fn bundled_version(adapter: &Path, augmented_path: Option<&str>) -> Option<(u64, u64, u64)> {
    let mut command = std::process::Command::new(adapter);
    command.args(["cli", "-V"]).env_remove("CODEX_PATH");
    if let Some(path) = augmented_path {
        command.env("PATH", path);
    }
    crate::cli_version::probe_command(&mut command)
}

pub fn validate_version(version: Option<(u64, u64, u64)>) -> Result<(), String> {
    if version.is_some_and(|version| QUALIFIED_VERSIONS.contains(&version)) {
        return Ok(());
    }
    let installed = version
        .map(|(a, b, c)| format!("{a}.{b}.{c}"))
        .unwrap_or_else(|| "unknown".into());
    Err(format!("Saved-connection Codex requires qualified CLI 0.153.4; installed version is {installed}. Hosted Codex is unaffected."))
}

pub fn clear_shell_snapshots(home: &Path) -> Result<(), String> {
    let snapshots = home.join("shell_snapshots");
    match std::fs::symlink_metadata(&snapshots) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Cannot inspect Codex shell snapshots".into()),
        Ok(meta) => {
            let result = if meta.is_dir() {
                std::fs::remove_dir_all(snapshots)
            } else {
                std::fs::remove_file(snapshots)
            };
            result.map_err(|_| "Cannot remove Codex shell snapshots from generated home".into())
        }
    }
}

pub fn catalog(model: &str, context: u64) -> Value {
    // Supply only capabilities demonstrated by function-tool qualification.
    // In particular do not inherit another model's freeform tools or metadata.
    json!({"models":[{
        "slug":model,"display_name":model,"description":"Saved connection",
        "supported_reasoning_levels":[],"default_reasoning_level":null,
        "shell_type":"default","visibility":"list","supported_in_api":true,
        "priority":0,"base_instructions":"You are a coding assistant.",
        "model_messages":null,"supports_reasoning_summaries":false,
        "support_verbosity":false,"default_verbosity":null,
        "apply_patch_tool_type":null,"web_search_tool_type":"text",
        "truncation_policy":{"mode":"bytes","limit":10000},
        "supports_parallel_tool_calls":false,"supports_search_tool":false,
        "context_window":context,"max_context_window":context,
        "auto_compact_token_limit":context.saturating_mul(3)/4,
        "input_modalities":["text"],"experimental_supported_tools":[],
        "used_fallback_model_metadata":false,"tool_mode":null,
        "multi_agent_version":null,"use_responses_lite":false,
        "prefer_websockets":false
    }]})
}

/// Native startup overrides win over workspace settings. Session MCP declarations
/// are supplied separately by ACP; this object never names user or project servers.
pub fn overrides(model: &str, context: u64) -> Value {
    json!({
        "model":model,"model_provider":"custom-gateway",
        "sandbox_mode":"workspace-write","approval_policy":"on-request",
        "model_context_window":context,
        "model_auto_compact_token_limit":context.saturating_mul(3)/4,
        "project_root_markers":[".git"],
        "features.shell_snapshot":false,"features.view_image":false,
        "features.apps":false,"features.plugins":false,"features.remote_plugin":false,
        "skills.include_instructions":false,"skills.bundled.enabled":false,
        "web_search":"disabled","allow_login_shell":false,
        "shell_environment_policy.inherit":"core",
        "shell_environment_policy.ignore_default_excludes":false,
        "shell_environment_policy.exclude":[PROVIDER_KEY_ENV],
        "shell_environment_policy.experimental_use_profile":false
    })
}

/// Only generated configuration belongs in the private home. The adapter's
/// providers/set handshake preserves this env_key while supplying base_url.
pub fn home_config(base_url: &str, catalog_path: &Path) -> Result<String, String> {
    let path = catalog_path
        .to_str()
        .ok_or("Codex metadata path is not UTF-8")?;
    let value = json!({
        "model_catalog_json":path,"project_root_markers":[".git"],
        "model_providers":{"custom-gateway":{
            "name":"Connection","base_url":base_url,"wire_api":"responses",
            "env_key":PROVIDER_KEY_ENV,"requires_openai_auth":false
        }}
    });
    toml::to_string(&value).map_err(|_| "Cannot encode native Codex configuration".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn bundled_probe_uses_the_adapters_native_cli_forwarding_route() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let adapter = root.path().join("codex-acp");
        std::fs::write(
            &adapter,
            "#!/bin/sh\n[ \"$1\" = cli ] && [ \"$2\" = -V ] || exit 1\necho 'codex-cli 0.153.4'\n",
        )
        .unwrap();
        std::fs::set_permissions(&adapter, std::fs::Permissions::from_mode(0o700)).unwrap();
        // No Codex executable exists on this PATH. The adapter resolves its own.
        assert_eq!(
            bundled_version(&adapter, Some(root.path().to_str().unwrap())),
            Some((0, 153, 4))
        );
    }

    #[test]
    fn unqualified_versions_name_installed_and_qualified_versions() {
        validate_version(Some((0, 153, 4))).unwrap();
        for version in [Some((0, 143, 0)), Some((0, 145, 0)), None] {
            let error = validate_version(version).unwrap_err();
            assert!(error.contains("0.153.4"));
            let actual = version
                .map(|(a, b, c)| format!("{a}.{b}.{c}"))
                .unwrap_or_else(|| "unknown".into());
            assert!(error.contains(&actual));
        }
    }

    #[test]
    fn older_snapshot_contents_are_removed_on_every_spawn() {
        let home = tempfile::tempdir().unwrap();
        for _ in 0..2 {
            std::fs::create_dir(home.path().join("shell_snapshots")).unwrap();
            std::fs::write(home.path().join("shell_snapshots/old.sh"), "SYNTHETIC_KEY").unwrap();
            clear_shell_snapshots(home.path()).unwrap();
            assert!(!home.path().join("shell_snapshots").exists());
        }
    }

    #[test]
    fn native_projection_is_exact_text_only_and_has_no_claimed_output_cap() {
        let model = "sonnet/model-[literal]";
        let catalog = catalog(model, 65536);
        assert_eq!(catalog["models"][0]["slug"], model);
        assert_eq!(catalog["models"][0]["input_modalities"], json!(["text"]));
        let config = overrides(model, 65536);
        assert_eq!(config["model"], model);
        assert_eq!(config["model_auto_compact_token_limit"], 49152);
        assert_eq!(config["project_root_markers"], json!([".git"]));
        assert_eq!(config["web_search"], "disabled");
        assert!(config.get("max_output_tokens").is_none());
        let home: toml::Table = toml::from_str(
            &home_config("http://localhost/v1", Path::new("/tmp/models.json")).unwrap(),
        )
        .unwrap();
        let provider = &home["model_providers"]["custom-gateway"];
        assert_eq!(provider["env_key"].as_str(), Some(PROVIDER_KEY_ENV));
        assert!(provider.get("api_key").is_none());
    }
}
