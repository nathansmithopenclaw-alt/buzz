use reqwest::{Client, Url};
use std::time::Duration;
pub(super) fn normalize_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let raw = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("http://{raw}")
    };
    let mut url = Url::parse(&raw).map_err(|_| "Enter a valid server address")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Use an HTTP or HTTPS address without embedded credentials".into());
    }
    if matches!(url.path(), "" | "/") {
        url.set_path("/v1");
    }
    Ok(url.to_string().trim_end_matches('/').to_string())
}
fn client(seconds: u64) -> Result<Client, String> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(seconds))
        .build()
        .map_err(|_| "Cannot initialize model connection".into())
}
async fn json(mut response: reqwest::Response) -> Result<serde_json::Value, String> {
    if !response.status().is_success() {
        return Err(format!(
            "Server returned HTTP {}. Check the address or API key.",
            response.status().as_u16()
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Cannot read server response")?
    {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err("Server response exceeds 1 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "Server did not return a supported model response".into())
}
pub(super) async fn catalog(
    base: &str,
    key: Option<&str>,
    seconds: u64,
) -> Result<Vec<String>, String> {
    let mut request = client(seconds)?.get(format!("{}/models", normalize_url(base)?));
    if let Some(key) = key {
        request = request.bearer_auth(key);
    }
    let value = json(
        request
            .send()
            .await
            .map_err(|_| "Server is unavailable from this computer")?,
    )
    .await?;
    let data = value
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or("Server does not expose an OpenAI-compatible model catalog")?;
    let mut models: Vec<_> = data
        .iter()
        .filter_map(|v| v.get("id").and_then(|v| v.as_str()))
        .filter(|id| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control))
        .take(512)
        .map(str::to_string)
        .collect();
    models.sort();
    models.dedup();
    if models.is_empty() {
        return Err("Server reported no models".into());
    }
    Ok(models)
}
pub(super) async fn test(base: &str, key: Option<&str>, model: &str) -> Result<(), String> {
    let mut request = client(30)?.post(format!("{}/chat/completions", normalize_url(base)?)).json(&serde_json::json!({"model":model,"messages":[{"role":"user","content":"Reply with OK."}],"max_tokens":16,"stream":false}));
    if let Some(key) = key {
        request = request.bearer_auth(key);
    }
    let value = json(
        request
            .send()
            .await
            .map_err(|_| "Model test timed out or server disconnected")?,
    )
    .await?;
    if value
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .is_none_or(|s| s.trim().is_empty())
    {
        return Err("Model did not return a text response".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn endpoint(
        status: &str,
        body: &str,
        extra: &str,
    ) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
            body.len()
        );
        let task = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = vec![0; 16384];
            let n = socket.read(&mut request).unwrap();
            let _ = socket.write_all(response.as_bytes());
            String::from_utf8_lossy(&request[..n]).into_owned()
        });
        (url, task)
    }
    #[tokio::test]
    async fn discovery_reads_catalog_only_and_deduplicates_models() {
        let (url, task) = endpoint(
            "200 OK",
            r#"{"data":[{"id":"Qwen/model"},{"id":"Qwen/model"}]}"#,
            "",
        );
        assert_eq!(catalog(&url, None, 2).await.unwrap(), vec!["Qwen/model"]);
        let request = task.join().unwrap();
        assert!(request.starts_with("GET /v1/models "));
        assert!(!request.to_lowercase().contains("authorization"));
        assert!(!request.contains("chat/completions"));
    }
    #[tokio::test]
    async fn credentials_stay_on_explicit_endpoint_and_errors_are_sanitized() {
        let (url, task) = endpoint("401 Unauthorized", "secret-server-body", "");
        let error = catalog(&url, Some("fixture-key"), 2).await.unwrap_err();
        assert!(error.contains("401"));
        assert!(!error.contains("secret-server-body"));
        assert!(task
            .join()
            .unwrap()
            .to_lowercase()
            .contains("authorization: bearer fixture-key"));
        let (url, task) = endpoint("302 Found", "", "Location: http://127.0.0.1:1/leak\r\n");
        assert!(catalog(&url, Some("fixture-key"), 2)
            .await
            .unwrap_err()
            .contains("302"));
        task.join().unwrap();
    }
    #[tokio::test]
    async fn unsupported_empty_and_oversized_catalogs_fail() {
        for body in [
            "{}".to_string(),
            r#"{"data":[]}"#.to_string(),
            "x".repeat(1024 * 1024 + 1),
        ] {
            let (url, task) = endpoint("200 OK", &body, "");
            assert!(catalog(&url, None, 2).await.is_err());
            task.join().unwrap();
        }
    }
    #[tokio::test]
    async fn explicit_model_test_requires_a_real_text_response() {
        let (url, task) = endpoint(
            "200 OK",
            r#"{"choices":[{"message":{"content":"OK"}}]}"#,
            "",
        );
        test(&url, None, "Qwen/model").await.unwrap();
        assert!(task
            .join()
            .unwrap()
            .starts_with("POST /v1/chat/completions "));
        let (url, task) = endpoint("200 OK", r#"{"choices":[{"message":{"content":""}}]}"#, "");
        assert!(test(&url, None, "Qwen/model").await.is_err());
        task.join().unwrap();
    }
    #[test]
    fn normalizes_common_addresses_without_leaking_credentials() {
        assert_eq!(
            normalize_url("localhost:11434").unwrap(),
            "http://localhost:11434/v1"
        );
        assert_eq!(
            normalize_url("https://server/custom/v1/").unwrap(),
            "https://server/custom/v1"
        );
        for bad in [
            "http://user:secret@server",
            "file:///tmp/model",
            "https://server?key=secret",
            "https://server/#secret",
        ] {
            assert!(normalize_url(bad).is_err());
        }
    }
}
