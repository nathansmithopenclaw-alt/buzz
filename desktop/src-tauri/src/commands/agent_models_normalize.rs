use std::collections::HashSet;

use crate::managed_agents::{AgentEffortInfo, AgentModelInfo, AgentModelsResponse};

/// Normalize raw `buzz-acp models --json` output into the frontend DTO.
pub(super) fn normalize_agent_models(
    raw: &serde_json::Value,
    persisted_model: Option<String>,
) -> AgentModelsResponse {
    let agent_name = raw["agent"]["name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let agent_version = raw["agent"]["version"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let mut models = Vec::new();
    let mut seen_ids = HashSet::new();

    if let Some(config_options) = raw["stable"]["configOptions"].as_array() {
        for option in config_options {
            if option.get("category").and_then(|value| value.as_str()) != Some("model") {
                continue;
            }
            for model in option
                .get("options")
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
            {
                let Some(id) = model.get("value").and_then(|value| value.as_str()) else {
                    continue;
                };
                if seen_ids.insert(id.to_string()) {
                    models.push(AgentModelInfo {
                        id: id.to_string(),
                        name: model
                            .get("displayName")
                            .or_else(|| model.get("name"))
                            .and_then(|value| value.as_str())
                            .map(str::to_string),
                        description: None,
                    });
                }
            }
        }
    }

    let mut agent_default_model = None;
    if let Some(unstable) = raw.get("unstable") {
        agent_default_model = unstable["currentModelId"].as_str().map(str::to_string);
        if let Some(available) = unstable["availableModels"].as_array() {
            for model in available {
                let Some(id) = model.get("modelId").and_then(|value| value.as_str()) else {
                    continue;
                };
                if seen_ids.insert(id.to_string()) {
                    models.push(AgentModelInfo {
                        id: id.to_string(),
                        name: model
                            .get("name")
                            .and_then(|value| value.as_str())
                            .map(str::to_string),
                        description: model
                            .get("description")
                            .and_then(|value| value.as_str())
                            .map(str::to_string),
                    });
                }
            }
        }
    }

    let effort_option = raw["stable"]["configOptions"]
        .as_array()
        .and_then(|options| {
            options.iter().find(|option| {
                option.get("category").and_then(|value| value.as_str()) == Some("thought_level")
                    || option
                        .get("id")
                        .or_else(|| option.get("configId"))
                        .and_then(|value| value.as_str())
                        == Some("effort")
            })
        });
    let effort_config_id = effort_option.and_then(|option| {
        option
            .get("id")
            .or_else(|| option.get("configId"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    });
    let effort_options = effort_option
        .and_then(|option| option.get("options"))
        .and_then(|value| value.as_array())
        .map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    Some(AgentEffortInfo {
                        id: option.get("value")?.as_str()?.to_string(),
                        name: option
                            .get("displayName")
                            .or_else(|| option.get("name"))
                            .and_then(|value| value.as_str())
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let fast_mode_option = raw["stable"]["configOptions"]
        .as_array()
        .and_then(|options| {
            options.iter().find(|option| {
                let id = option
                    .get("id")
                    .or_else(|| option.get("configId"))
                    .and_then(|value| value.as_str());
                id == Some("fast")
                    || id == Some("fast-mode")
                    || (option.get("category").and_then(|value| value.as_str())
                        == Some("model_config")
                        && option
                            .get("name")
                            .or_else(|| option.get("displayName"))
                            .and_then(|value| value.as_str())
                            .is_some_and(|name| name.eq_ignore_ascii_case("fast mode")))
            })
        });
    let fast_mode_config_id = fast_mode_option.and_then(|option| {
        option
            .get("id")
            .or_else(|| option.get("configId"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    });
    let fast_mode_enabled = fast_mode_option.and_then(|option| {
        option.get("currentValue").and_then(|value| {
            value
                .as_bool()
                .or_else(|| value.as_str().and_then(|value| match value {
                    "on" => Some(true),
                    "off" => Some(false),
                    _ => None,
                }))
        })
    });
    let fast_mode_description = fast_mode_option.and_then(|option| {
        option
            .get("description")
            .and_then(|value| value.as_str())
            .map(str::to_string)
    });

    AgentModelsResponse {
        agent_name,
        agent_version,
        supports_switching: !models.is_empty(),
        models,
        agent_default_model,
        selected_model: persisted_model,
        effort_config_id,
        effort_options,
        fast_mode_config_id,
        fast_mode_enabled,
        fast_mode_description,
    }
}
