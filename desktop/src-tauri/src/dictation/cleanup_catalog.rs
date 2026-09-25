//! Cleanup catalog metadata; stored credentials never cross IPC.

use crate::app_state::AppState;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::{AppHandle, State};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CleanupCatalog {
    agent_open_router: bool,
    open_router: Vec<ModelChoice>,
    local_connection_ids: Vec<String>,
    error: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct ModelChoice {
    id: String,
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct CatalogResponse {
    data: Vec<RouterModel>,
}
#[derive(Deserialize)]
struct RouterModel {
    #[serde(flatten)]
    choice: ModelChoice,
    architecture: Option<Architecture>,
}
#[derive(Deserialize)]
struct Architecture {
    output_modalities: Vec<String>,
}

async fn openrouter(client: &reqwest::Client, key: &str) -> Result<Vec<ModelChoice>, String> {
    let mut response = client
        .get(format!("{}/models", super::OPENROUTER_BASE_URL))
        .bearer_auth(key)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|_| "Could not load OpenRouter models. Try again.")?;
    if !response.status().is_success() {
        return Err(format!(
            "Could not load OpenRouter models (HTTP {})",
            response.status()
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read OpenRouter models")?
    {
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            return Err("OpenRouter model list is too large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let catalog: CatalogResponse =
        serde_json::from_slice(&bytes).map_err(|_| "OpenRouter returned an invalid model list")?;
    Ok(text_models(catalog))
}

fn text_models(catalog: CatalogResponse) -> Vec<ModelChoice> {
    catalog
        .data
        .into_iter()
        .filter(|entry| {
            entry
                .architecture
                .as_ref()
                .is_some_and(|a| a.output_modalities.iter().any(|m| m == "text"))
        })
        .map(|entry| entry.choice)
        .collect()
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
    let (open_router, error) = match key {
        Some(key) => match openrouter(&state.http_client, key).await {
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
mod tests {
    #[test]
    fn cleanup_accepts_text_models_without_tool_support() {
        let response = serde_json::from_str(r#"{"data":[{"id":"text-only","architecture":{"output_modalities":["text"]}},{"id":"image","architecture":{"output_modalities":["image"]}}]}"#).unwrap();
        let models = super::text_models(response);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "text-only");
    }
}
