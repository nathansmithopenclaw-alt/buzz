use super::*;
use axum::{http::HeaderMap, routing::get, Router};

async fn server(body: &'static str, status: u16) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/models",
                get(move |headers: HeaderMap| async move {
                    assert_eq!(
                        headers.get("authorization").unwrap(),
                        "Bearer test-catalog-key"
                    );
                    (axum::http::StatusCode::from_u16(status).unwrap(), body)
                }),
            ),
        )
        .await
        .unwrap();
    });
    (format!("http://{address}"), task)
}

fn env(base: String) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("OPENROUTER_API_KEY".into(), "test-catalog-key".into()),
        ("OPENROUTER_BASE_URL".into(), base),
    ])
}

#[tokio::test]
async fn cleanup_and_agents_share_transport_but_keep_their_capability_filters() {
    let (base, task) = server(r#"{"data":[
        {"id":"text-only","name":"Text model","architecture":{"output_modalities":["text"]}},
        {"id":"tools-text","supported_parameters":["tools"],"architecture":{"output_modalities":["text"]}},
        {"id":"image-tools","supported_parameters":["tools"],"architecture":{"output_modalities":["image"]}},
        {"id":"legacy-tools","supported_parameters":["tools"],"architecture":{}}
    ]}"#, 200).await;
    let env = env(base);
    let client = reqwest::Client::new();
    let cleanup = openrouter(&client, &env).await.unwrap();
    assert_eq!(
        cleanup.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["text-only", "tools-text"]
    );
    assert_eq!(cleanup[0].name, "Text model");
    let provider = effective_discovery_provider(Some("openrouter"), None, &env);
    let agents = discover_openrouter_models(
        &client,
        &provider,
        &env,
        Some("saved-model".into()),
        OpenRouterModelFilter::Tools,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        agents
            .models
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["tools-text", "image-tools", "legacy-tools"]
    );
    assert_eq!(agents.selected_model.as_deref(), Some("saved-model"));
    task.abort();
}

#[tokio::test]
async fn cleanup_uses_shared_http_error_redaction() {
    let (base, task) = server("rejected test-catalog-key", 401).await;
    let error = openrouter(&reqwest::Client::new(), &env(base))
        .await
        .err()
        .unwrap();
    assert!(error.contains("401"), "{error}");
    assert!(!error.contains("test-catalog-key"), "{error}");
    task.abort();
}
