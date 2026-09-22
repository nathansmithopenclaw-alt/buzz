//! Desktop-managed Claude ACP model catalog.
//!
//! claude-agent-acp reads project settings from the ACP session cwd. Managed
//! agents intentionally share the Buzz nest as that cwd, so this artifact is a
//! harness-wide catalog rather than per-agent state. It affects any Claude
//! session launched with `~/.buzz` as cwd (including a hypothetical terminal
//! session started there), but contains the comprehensive catalog and does not
//! select a model; the saved per-agent `model` remains authoritative.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use tauri::AppHandle;

const BASE_ALIASES: &[&str] = &["opus", "sonnet", "haiku"];

/// SDK-recognizable allowlist entry and optional API target override. Keeping
/// the recognizable entry lets the adapter carry capability metadata (native
/// effort levels) onto dated targets.
const CURATED: &[(&str, Option<&str>, Option<&str>)] = &[
    ("claude-fable-5-1", None, None),
    ("fable", Some("claude-fable-5"), None),
    ("claude-opus-5-5", None, Some("2.1.280")),
    ("claude-opus-5", None, None),
    ("claude-opus-4-8", None, None),
    ("claude-opus-4-7", None, None),
    ("claude-opus-4-6", None, None),
    ("claude-opus-4-5", Some("claude-opus-4-5-20251101"), None),
    ("claude-opus-4-1", Some("claude-opus-4-1-20250805"), None),
    ("claude-opus-4", Some("claude-opus-4-20250514"), None),
    ("claude-sonnet-5", None, None),
    ("claude-sonnet-4-6", None, None),
    (
        "claude-sonnet-4-5",
        Some("claude-sonnet-4-5-20250929"),
        None,
    ),
    ("claude-sonnet-4", Some("claude-sonnet-4-20250514"), None),
    ("claude-haiku-4-5", Some("claude-haiku-4-5-20251001"), None),
];

pub(crate) fn minimum_cli_version(model: &str) -> Option<&'static str> {
    CURATED.iter().find_map(|(entry, target, minimum)| {
        (*entry == model || target.is_some_and(|target| target == model))
            .then_some(*minimum)
            .flatten()
    })
}

pub(crate) fn sync_claude_model_catalog(
    nest: &Path,
    saved_models: impl IntoIterator<Item = String>,
) -> Result<(), String> {
    let settings_dir = nest.join(".claude");
    std::fs::create_dir_all(&settings_dir)
        .map_err(|error| format!("create {}: {error}", settings_dir.display()))?;
    let path = settings_dir.join("settings.json");
    let mut root = if path.exists() {
        let bytes =
            std::fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .map_err(|error| format!("parse {}: {error}", path.display()))?
    } else {
        serde_json::json!({})
    };
    let object = root
        .as_object_mut()
        .ok_or_else(|| format!("{} must contain a JSON object", path.display()))?;

    let mut available: BTreeSet<String> =
        BASE_ALIASES.iter().map(|value| value.to_string()).collect();
    let mut overrides = BTreeMap::<String, String>::new();
    let mut advertised_targets = BTreeSet::<String>::new();
    for (entry, target, _) in CURATED {
        available.insert((*entry).to_string());
        advertised_targets.insert(target.unwrap_or(entry).to_string());
        if let Some(target) = target {
            overrides.insert((*entry).to_string(), (*target).to_string());
        }
    }

    for saved in saved_models {
        let trimmed = saved.trim();
        if trimmed.is_empty()
            || advertised_targets.contains(trimmed)
            || is_supported_builtin_qualifier(trimmed)
        {
            continue;
        }
        available.insert(trimmed.to_string());
    }

    object.insert(
        "availableModels".to_string(),
        serde_json::to_value(available).map_err(|error| error.to_string())?,
    );
    object.insert(
        "modelOverrides".to_string(),
        serde_json::to_value(overrides).map_err(|error| error.to_string())?,
    );
    let payload = serde_json::to_vec_pretty(&root)
        .map_err(|error| format!("serialize Claude model catalog: {error}"))?;
    super::storage::atomic_write_json(&path, &payload)
}

pub(crate) fn sync_claude_catalog_for_state(
    nest: &Path,
    records: &[super::ManagedAgentRecord],
    definitions: &[super::AgentDefinition],
) -> Result<(), String> {
    let mut models = Vec::new();
    for definition in definitions {
        if definition.runtime.as_deref() == Some("claude") {
            if let Some(model) = definition.model.clone() {
                models.push(model);
            }
        }
    }
    for record in records {
        let command = super::record_agent_command(record, definitions);
        if super::known_acp_runtime(&command).is_some_and(|runtime| runtime.id == "claude") {
            if let Some(model) = record.model.clone() {
                models.push(model);
            }
        }
    }
    sync_claude_model_catalog(nest, models)
}

pub(crate) fn sync_catalog_and_save_agents(
    app: &AppHandle,
    records: &[super::ManagedAgentRecord],
    definitions: &[super::AgentDefinition],
) -> Result<(), String> {
    if let Some(nest) = super::nest_dir() {
        sync_claude_catalog_for_state(&nest, records, definitions)?;
    }
    super::storage::save_managed_agents(app, records)
}

fn is_supported_builtin_qualifier(model: &str) -> bool {
    model
        .strip_suffix("[1m]")
        .is_some_and(|base| matches!(base, "opus" | "sonnet" | "haiku"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_preserves_unknown_keys_and_keeps_existing_agent_models_resolvable() {
        let temp = tempfile::tempdir().unwrap();
        let claude = temp.path().join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::write(
            claude.join("settings.json"),
            r#"{"permissions":{"defaultMode":"plan"}}"#,
        )
        .unwrap();

        sync_claude_model_catalog(
            temp.path(),
            [
                "claude-fable-5".to_string(),
                "opus[1m]".to_string(),
                "vendor-custom".to_string(),
            ],
        )
        .unwrap();

        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(claude.join("settings.json")).unwrap()).unwrap();
        assert_eq!(value["permissions"]["defaultMode"], "plan");
        let available = value["availableModels"].as_array().unwrap();
        assert!(available.iter().any(|value| value == "vendor-custom"));
        assert!(available.iter().any(|value| value == "opus"));
        assert!(!available.iter().any(|value| value == "opus[1m]"));
        assert_eq!(value["modelOverrides"]["fable"], "claude-fable-5");
        assert_eq!(
            value["modelOverrides"]["claude-opus-4-1"],
            "claude-opus-4-1-20250805"
        );

        let advertised: BTreeSet<&str> = available
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(|entry| {
                value["modelOverrides"]
                    .get(entry)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(entry)
            })
            .collect();
        let expected = BTreeSet::from([
            "opus",
            "sonnet",
            "haiku",
            "claude-fable-5",
            "claude-fable-5-1",
            "claude-opus-5-5",
            "claude-opus-5",
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-opus-4-6",
            "claude-opus-4-5-20251101",
            "claude-opus-4-1-20250805",
            "claude-opus-4-20250514",
            "claude-sonnet-5",
            "claude-sonnet-4-6",
            "claude-sonnet-4-5-20250929",
            "claude-sonnet-4-20250514",
            "claude-haiku-4-5-20251001",
            "vendor-custom",
        ]);
        assert_eq!(advertised, expected);
        assert!(
            advertised.contains("claude-fable-5"),
            "Helm's saved Fable model must resolve"
        );
        assert!(
            advertised.contains("opus"),
            "Anvil's opus[1m] value canonicalizes to an advertised opus alias"
        );
    }

    #[test]
    fn minimum_cli_version_is_only_declared_for_verified_model() {
        assert_eq!(minimum_cli_version("claude-opus-5-5"), Some("2.1.280"));
        assert_eq!(minimum_cli_version("claude-opus-5"), None);
        assert_eq!(minimum_cli_version("opus"), None);
    }
}
