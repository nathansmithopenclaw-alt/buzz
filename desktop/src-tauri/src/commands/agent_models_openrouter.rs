use std::{collections::BTreeMap, time::Duration};

use serde::Deserialize;

use crate::managed_agents::{AgentModelInfo, AgentModelsResponse};

#[cfg(test)]
use super::env_value;
use super::{env_or_process_value, redaction_env_with_value, DiscoveryProvider};

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(Clone))]
pub(super) struct OpenRouterModelListResponse {
    pub data: Vec<OpenRouterModelListItem>,
}

#[derive(Debug, Default, Deserialize)]
#[cfg_attr(test, derive(Clone))]
pub(super) struct OpenRouterModelListItem {
    pub id: String,
    #[serde(default)]
    pub supported_parameters: Vec<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub architecture: Option<OpenRouterArchitecture>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(Clone))]
pub(super) struct OpenRouterArchitecture {
    #[serde(default)]
    pub output_modalities: Vec<String>,
}

/// Required capability of a model catalog consumer.
#[derive(Clone, Copy)]
pub(crate) enum OpenRouterModelFilter {
    Tools,
    Text,
}

pub(super) fn is_openrouter_provider(provider: Option<&str>) -> bool {
    matches!(
        provider
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("openrouter")
    )
}

#[cfg(test)]
pub(super) fn openrouter_models_url(env: &BTreeMap<String, String>) -> String {
    let base_url = env_value(env, "OPENROUTER_BASE_URL")
        .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string());
    format!("{}/models", base_url.trim_end_matches('/'))
}

fn openrouter_models_url_for_discovery(env: &BTreeMap<String, String>) -> String {
    let base_url = env_or_process_value(env, "OPENROUTER_BASE_URL")
        .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string());
    format!("{}/models", base_url.trim_end_matches('/'))
}

/// Shared authenticated, bounded OpenRouter catalog discovery.
pub(crate) async fn discover_openrouter_models(
    client: &reqwest::Client,
    provider: &DiscoveryProvider,
    env: &BTreeMap<String, String>,
    selected_model: Option<String>,
    capability: OpenRouterModelFilter,
) -> Result<Option<AgentModelsResponse>, String> {
    if !is_openrouter_provider(provider.as_deref()) {
        return Ok(None);
    }

    let api_key = match provider.required_env(env, "OPENROUTER_API_KEY")? {
        Some(api_key) => api_key,
        None => return Ok(None),
    };
    let redaction_env = redaction_env_with_value(env, "OPENROUTER_API_KEY", &api_key);
    let url = openrouter_models_url_for_discovery(env);
    let mut response = client
        .get(&url)
        .bearer_auth(&api_key)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|error| format!("OpenRouter model discovery request failed: {error}"))?;
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("OpenRouter model discovery response read failed: {error}"))?
    {
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            return Err("OpenRouter model list is too large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let body = String::from_utf8_lossy(&bytes);
        let body = crate::managed_agents::redact_env_values_in(&body, &redaction_env);
        return Err(format!("OpenRouter model discovery HTTP {status}: {body}"));
    }
    let response = serde_json::from_slice::<OpenRouterModelListResponse>(&bytes)
        .map_err(|error| format!("OpenRouter model discovery response parse failed: {error}"))?;

    filter_openrouter_models(response, selected_model, capability)
}

pub(super) fn filter_openrouter_models(
    response: OpenRouterModelListResponse,
    selected_model: Option<String>,
    capability: OpenRouterModelFilter,
) -> Result<Option<AgentModelsResponse>, String> {
    let models: Vec<AgentModelInfo> = response
        .data
        .into_iter()
        .filter(|m| match capability {
            OpenRouterModelFilter::Tools => m.supported_parameters.iter().any(|p| p == "tools"),
            OpenRouterModelFilter::Text => m
                .architecture
                .as_ref()
                .is_some_and(|a| a.output_modalities.iter().any(|m| m == "text")),
        })
        .map(|m| AgentModelInfo {
            id: m.id.clone(),
            name: Some(match capability {
                OpenRouterModelFilter::Tools => m.id,
                OpenRouterModelFilter::Text => m.name.unwrap_or(m.id),
            }),
            description: None,
        })
        .collect();

    if models.is_empty() {
        let capability = match capability {
            OpenRouterModelFilter::Tools => "tools",
            OpenRouterModelFilter::Text => "text",
        };
        return Err(format!(
            "OpenRouter model discovery returned no {capability}-capable models"
        ));
    }

    Ok(Some(AgentModelsResponse {
        agent_name: "openrouter".to_string(),
        agent_version: "models-api".to_string(),
        models,
        agent_default_model: None,
        selected_model,
        supports_switching: true,
        effort_config_id: None,
        effort_options: Vec::new(),
        ..AgentModelsResponse::default()
    }))
}
