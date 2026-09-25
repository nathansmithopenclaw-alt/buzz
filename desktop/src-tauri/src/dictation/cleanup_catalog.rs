//! Cleanup catalog metadata; stored credentials never cross IPC.

use crate::app_state::AppState;
use crate::commands::{
    discover_openrouter_models, effective_discovery_provider, OpenRouterModelFilter,
};
use serde::Serialize;
use std::collections::BTreeMap;
use tauri::{AppHandle, State};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CleanupCatalog {
    agent_open_router: bool,
    open_router: Vec<ModelChoice>,
    local_connection_ids: Vec<String>,
    error: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct ModelChoice {
    id: String,
    #[serde(default)]
    name: String,
}

async fn openrouter(
    client: &reqwest::Client,
    env: &BTreeMap<String, String>,
) -> Result<Vec<ModelChoice>, String> {
    let provider = effective_discovery_provider(Some("openrouter"), None, env);
    let models =
        discover_openrouter_models(client, &provider, env, None, OpenRouterModelFilter::Text)
            .await?
            .ok_or("OpenRouter is not configured")?;
    Ok(models
        .models
        .into_iter()
        .map(|model| ModelChoice {
            name: model.name.unwrap_or_else(|| model.id.clone()),
            id: model.id,
        })
        .collect())
}

/// Lists text-capable OpenRouter models and local connection IDs.
#[tauri::command]
pub async fn dictation_cleanup_catalog(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CleanupCatalog, String> {
    let config = crate::managed_agents::global_config::load_global_agent_config(&app)?;
    let key = super::agent_openrouter_key(&config.env_vars);
    let local_connection_ids = crate::model_connections::list_model_connections(app)?
        .into_iter()
        .filter(|connection| crate::model_connections::is_local_model_host(&connection.base_url))
        .map(|connection| connection.id)
        .collect();
    // Match the endpoint used by dictation cleanup, independent of agent overrides.
    let mut env = config.env_vars.clone();
    env.insert(
        "OPENROUTER_BASE_URL".into(),
        super::OPENROUTER_BASE_URL.into(),
    );
    let (open_router, error) = match key {
        Some(_) => match openrouter(&state.http_client, &env).await {
            Ok(models) => (models, None),
            Err(error) => (Vec::new(), Some(error)),
        },
        None => (Vec::new(), None),
    };
    Ok(CleanupCatalog {
        agent_open_router: key.is_some(),
        open_router,
        local_connection_ids,
        error,
    })
}

#[cfg(test)]
#[path = "cleanup_catalog_tests.rs"]
mod tests;
