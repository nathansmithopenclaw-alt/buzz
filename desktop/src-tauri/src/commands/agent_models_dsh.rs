//! Endpoint catalog discovery must not start dsh: its profile needs a selected model.
use super::{normalize_openai_compatible_models, AgentModelsResponse, OpenAiModelListResponse};
use std::collections::BTreeMap;

pub(super) async fn discover(
    command: &str,
    env: &BTreeMap<String, String>,
    selected_model: Option<String>,
) -> Result<Option<AgentModelsResponse>, String> {
    if crate::managed_agents::known_acp_runtime(command).map(|m| m.id) != Some("deepseek") {
        return Ok(None);
    }
    let (base, key) = route(env)?;
    let url = reqwest::Url::parse(&base)
        .map_err(|_| "The model endpoint must be an HTTP or HTTPS URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Use an HTTP or HTTPS endpoint without credentials, query, or fragment".into());
    }
    // Never follow redirects carrying endpoint credentials to another service.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|_| "Could not initialize model discovery")?;
    let mut request = client.get(format!("{}/models", base.trim_end_matches('/')));
    if let Some(key) = key {
        request = request.bearer_auth(key);
    }
    let response = request.send().await.map_err(|_| {
        "Could not reach the selected model endpoint; check its URL and availability"
    })?;
    if !response.status().is_success() {
        return Err(format!(
            "Model discovery returned HTTP {}. Check the selected endpoint and its API key.",
            response.status()
        ));
    }
    // Bound the response even when Content-Length is absent or misleading.
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read the model catalog")?
    {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err("Model catalog exceeds the 1 MiB limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let raw: OpenAiModelListResponse = serde_json::from_slice(&bytes)
        .map_err(|_| "The endpoint did not return an OpenAI-compatible model catalog")?;
    let models = normalize_openai_compatible_models(raw, None);
    if models.is_empty() {
        return Err("The selected endpoint reported no models".into());
    }
    Ok(Some(AgentModelsResponse {
        agent_name: "DeepSeek Harness".into(),
        agent_version: "models-api".into(),
        models,
        selected_model,
        supports_switching: true,
        ..AgentModelsResponse::default()
    }))
}

fn route(env: &BTreeMap<String, String>) -> Result<(String, Option<String>), String> {
    let value = |key| {
        super::super::agent_models_env::env_value_or_process_if_absent(env, key)
            .filter(|v| !v.is_empty())
    };
    let (base, key) = match value("DSH_PROVIDER").as_deref() {
        Some("deepseek-hosted") => ("https://api.deepseek.com".to_string(),
            Some(value("DEEPSEEK_API_KEY").ok_or("Enter a DeepSeek API key to load hosted models")?)),
        Some("deepseek-compatible") => (value("DSH_LLM_BASE_URL")
            .ok_or("Enter the local / OpenAI-compatible endpoint URL (DSH_LLM_BASE_URL) to load models")?, value("DSH_API_KEY")),
        _ => return Err("Choose DeepSeek API or a local / OpenAI-compatible endpoint to load models".into()),
    };
    Ok((base, key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    fn endpoint(status: &str, body: &str) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fixture");
        let url = format!("http://{}/v1", listener.local_addr().expect("address"));
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .expect("timeout");
            let mut buf = [0; 8192];
            let n = stream.read(&mut buf).expect("request");
            stream.write_all(response.as_bytes()).expect("response");
            String::from_utf8_lossy(&buf[..n]).to_string()
        });
        (url, handle)
    }

    #[test]
    fn hosted_route_ignores_local_endpoint_and_local_token() {
        let env = BTreeMap::from([
            ("DSH_PROVIDER".into(), "deepseek-hosted".into()),
            ("DEEPSEEK_API_KEY".into(), "hosted-fixture".into()),
            ("DSH_API_KEY".into(), "local-fixture".into()),
            ("DSH_LLM_BASE_URL".into(), "http://localhost:1/v1".into()),
        ]);
        assert_eq!(
            route(&env).expect("hosted route"),
            (
                "https://api.deepseek.com".into(),
                Some("hosted-fixture".into())
            )
        );
    }

    #[tokio::test]
    async fn dsh_catalog_before_model_selection_uses_exact_endpoint_and_token() {
        let (url, server) = endpoint(
            "200 OK",
            r#"{"data":[{"id":"Qwen/Qwen3.8-27B"},{"id":"Qwen/Qwen3.8-27B"},{"id":"another-model"}]}"#,
        );
        let env = BTreeMap::from([
            ("DSH_PROVIDER".into(), "deepseek-compatible".into()),
            ("DSH_LLM_BASE_URL".into(), url),
            ("DSH_API_KEY".into(), "fixture-local-token".into()),
            ("DEEPSEEK_API_KEY".into(), "must-not-leak".into()),
        ]);
        let result = discover("/local/bin/dsh", &env, None)
            .await
            .expect("catalog")
            .expect("dsh");
        assert_eq!(result.models.len(), 2);
        assert!(result.models.iter().any(|m| m.id == "Qwen/Qwen3.8-27B"));
        assert!(result.selected_model.is_none());
        assert!(result.supports_switching);
        let request = server.join().expect("server");
        assert!(request.starts_with("GET /v1/models "));
        assert!(request.contains("Bearer fixture-local-token"));
        assert!(!request.contains("must-not-leak"));
    }

    #[tokio::test]
    async fn dsh_catalog_errors_do_not_fall_back_or_echo_response_credentials() {
        let (url, server) = endpoint("401 Unauthorized", "fixture-secret-echo");
        let env = BTreeMap::from([
            ("DSH_PROVIDER".into(), "deepseek-compatible".into()),
            ("DSH_LLM_BASE_URL".into(), url),
            ("DSH_API_KEY".into(), String::new()),
        ]);
        let error = discover("dsh", &env, Some("saved-model".into()))
            .await
            .expect_err("401");
        assert!(error.contains("401"));
        assert!(!error.contains("fixture-secret-echo"));
        assert!(!server.join().expect("server").contains("authorization:"));
        assert!(discover(
            "dsh",
            &BTreeMap::from([("DSH_PROVIDER".into(), String::new())]),
            None
        )
        .await
        .is_err());
        assert!(discover("codex-acp", &env, None)
            .await
            .expect("other harness")
            .is_none());
    }
}
