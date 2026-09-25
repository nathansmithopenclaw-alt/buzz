//! Cached endpoint evidence from the explicit Test action, never picker rendering.
use super::probe;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const REVISION: u32 = 1;
const TOOL: &str = "connection_probe";
const NONCE: &str = "connection-ok";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    ChatCompletions,
    Responses,
    AnthropicMessages,
}

impl Protocol {
    fn path(&self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat/completions",
            Self::Responses => "responses",
            Self::AnthropicMessages => "messages",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtocolCheck {
    pub protocol: Protocol,
    pub tool_round_trip: bool,
    pub reason: Option<String>,
    /// Accepted wire parameters, intersected with the harness's own vocabulary.
    #[serde(default)]
    pub effort_levels: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Qualification {
    pub revision: u32,
    pub model: String,
    pub context_window: Option<u64>,
    pub output_token_limit: Option<u64>,
    pub protocols: Vec<ProtocolCheck>,
}

impl Qualification {
    pub fn check(&self, model: &str, protocol: Protocol) -> Option<&ProtocolCheck> {
        (self.revision == REVISION && self.model == model)
            .then(|| self.protocols.iter().find(|row| row.protocol == protocol))
            .flatten()
    }
}

/// Output reserve is explicit and proportional to a reported context window.
/// Unknown limits remain unknown; a hosted-model name never supplies a limit.
pub fn output_cap(context: u64) -> u64 {
    (context / 8).clamp(256, 8192).min(context / 2)
}

pub(super) async fn qualify(
    base: &str,
    key: Option<&str>,
    model: &str,
    context_window: Option<u64>,
) -> Qualification {
    let mut protocols = Vec::new();
    for protocol in [
        Protocol::ChatCompletions,
        Protocol::Responses,
        Protocol::AnthropicMessages,
    ] {
        let result = round_trip(base, key, model, &protocol).await;
        let mut row = ProtocolCheck {
            protocol,
            tool_round_trip: result.is_ok(),
            reason: result.err(),
            effort_levels: Vec::new(),
        };
        if row.tool_round_trip && row.protocol == Protocol::AnthropicMessages {
            // ACP advertises the CLI's vocabulary, not the endpoint's accepted
            // values. Qualify these independently, one bounded request at a time.
            for level in ["low", "medium", "high", "xhigh", "max"] {
                let payload = json!({"model":model,"max_tokens":16,"stream":false,
                    "messages":[{"role":"user","content":"Reply OK."}],
                    "output_config":{"effort":level}});
                if request(base, key, &row.protocol, payload).await.is_ok() {
                    row.effort_levels.push(level.into());
                }
            }
        }
        protocols.push(row);
    }
    Qualification {
        revision: REVISION,
        model: model.into(),
        context_window,
        output_token_limit: context_window.map(output_cap),
        protocols,
    }
}

async fn request(
    base: &str,
    key: Option<&str>,
    protocol: &Protocol,
    payload: Value,
) -> Result<Vec<u8>, String> {
    let mut request = probe::client(20)?
        .post(format!(
            "{}/{}",
            probe::normalize_url(base)?,
            protocol.path()
        ))
        .header("anthropic-version", "2023-06-01")
        .json(&payload);
    if let Some(key) = key {
        request = request.bearer_auth(key);
        if *protocol == Protocol::AnthropicMessages {
            request = request.header("x-api-key", key);
        }
    }
    let response = request
        .send()
        .await
        .map_err(|_| "Server timed out or disconnected during qualification")?;
    probe::response_bytes(response).await
}

#[derive(Default)]
struct ToolCall {
    id: String,
    name: String,
    arguments: String,
}

fn streamed_tool(bytes: &[u8], protocol: &Protocol) -> Result<ToolCall, String> {
    let mut call = ToolCall::default();
    let mut seen = false;
    for line in std::str::from_utf8(bytes)
        .map_err(|_| "Invalid streaming response")?
        .lines()
    {
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        if data == "[DONE]" {
            continue;
        }
        let event: Value = serde_json::from_str(data).map_err(|_| "Invalid streaming event")?;
        match protocol {
            Protocol::ChatCompletions => {
                if let Some(calls) = event
                    .pointer("/choices/0/delta/tool_calls")
                    .and_then(Value::as_array)
                {
                    for part in calls {
                        if part.get("index").and_then(Value::as_u64).unwrap_or(0) != 0 {
                            return Err("Model returned more than one probe tool call".into());
                        }
                        if let Some(id) = part.get("id").and_then(Value::as_str) {
                            call.id = id.into();
                        }
                        if let Some(name) = part.pointer("/function/name").and_then(Value::as_str) {
                            call.name.push_str(name);
                        }
                        if let Some(args) =
                            part.pointer("/function/arguments").and_then(Value::as_str)
                        {
                            call.arguments.push_str(args);
                        }
                        seen = true;
                    }
                }
            }
            Protocol::Responses => {
                if let Some(item) = event.get("item").filter(|v| v["type"] == "function_call") {
                    if let Some(id) = item.get("call_id").and_then(Value::as_str) {
                        call.id = id.into();
                    }
                    if let Some(name) = item.get("name").and_then(Value::as_str) {
                        call.name = name.into();
                    }
                    if let Some(args) = item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .filter(|v| !v.is_empty())
                    {
                        call.arguments = args.into();
                    }
                    seen = true;
                }
                if event["type"] == "response.function_call_arguments.delta" {
                    if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                        call.arguments.push_str(delta);
                    }
                }
                if event["type"] == "response.function_call_arguments.done" {
                    if let Some(args) = event.get("arguments").and_then(Value::as_str) {
                        call.arguments = args.into();
                    }
                }
            }
            Protocol::AnthropicMessages => {
                if let Some(block) = event
                    .get("content_block")
                    .filter(|v| v["type"] == "tool_use")
                {
                    call.id = block["id"].as_str().unwrap_or_default().into();
                    call.name = block["name"].as_str().unwrap_or_default().into();
                    seen = true;
                }
                if event.pointer("/delta/type").and_then(Value::as_str) == Some("input_json_delta")
                {
                    if let Some(delta) =
                        event.pointer("/delta/partial_json").and_then(Value::as_str)
                    {
                        call.arguments.push_str(delta);
                    }
                }
            }
        }
    }
    let arguments: Value = serde_json::from_str(&call.arguments)
        .map_err(|_| "Model did not stream valid tool arguments")?;
    if !seen || call.id.is_empty() || call.name != TOOL || arguments["value"] != NONCE {
        return Err("Model did not stream the requested tool call".into());
    }
    Ok(call)
}

async fn round_trip(
    base: &str,
    key: Option<&str>,
    model: &str,
    protocol: &Protocol,
) -> Result<(), String> {
    let schema = json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false});
    let prompt = format!(
        "Call {TOOL} with value {NONCE}. After the tool returns, reply with its result only."
    );
    let user = json!({"role":"user","content":prompt});
    let function =
        json!({"name":TOOL,"description":"Echo a connection test value", "parameters":schema});
    let mut payload = match protocol {
        Protocol::ChatCompletions => {
            json!({"model":model,"messages":[user],"tools":[{"type":"function","function":function}],"tool_choice":{"type":"function","function":{"name":TOOL}},"max_tokens":512,"stream":true})
        }
        Protocol::Responses => {
            json!({"model":model,"input":[user],"tools":[{"type":"function","name":TOOL,"description":"Echo a connection test value","parameters":schema}],"tool_choice":{"type":"function","name":TOOL},"max_output_tokens":512,"stream":true})
        }
        Protocol::AnthropicMessages => {
            json!({"model":model,"messages":[user],"tools":[{"name":TOOL,"description":"Echo a connection test value","input_schema":schema}],"tool_choice":{"type":"tool","name":TOOL},"max_tokens":512,"stream":true})
        }
    };
    let call = streamed_tool(
        &request(base, key, protocol, payload.clone()).await?,
        protocol,
    )?;
    payload["stream"] = json!(false);
    payload["tool_choice"] = json!("none");
    match protocol {
        Protocol::ChatCompletions => {
            payload["messages"] = json!([user,{"role":"assistant","content":null,"tool_calls":[{"id":call.id,"type":"function","function":{"name":call.name,"arguments":call.arguments}}]},{"role":"tool","tool_call_id":call.id,"content":NONCE}])
        }
        Protocol::Responses => {
            payload["input"] = json!([user,{"type":"function_call","call_id":call.id,"name":call.name,"arguments":call.arguments},{"type":"function_call_output","call_id":call.id,"output":NONCE}])
        }
        Protocol::AnthropicMessages => {
            payload["tool_choice"] = json!({"type":"none"});
            let input: Value =
                serde_json::from_str(&call.arguments).map_err(|_| "Invalid tool arguments")?;
            payload["messages"] = json!([user,{"role":"assistant","content":[{"type":"tool_use","id":call.id,"name":call.name,"input":input}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":call.id,"content":NONCE}]}]);
        }
    }
    let bytes = request(base, key, protocol, payload).await?;
    let response: Value =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid tool result response")?;
    let text = match protocol {
        Protocol::ChatCompletions => response
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        Protocol::Responses => response
            .get("output")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("content").and_then(Value::as_array))
            .flatten()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect(),
        Protocol::AnthropicMessages => response
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect(),
    };
    if !text.contains(NONCE) {
        return Err("Model did not answer after the tool result".into());
    }
    Ok(())
}

pub(super) fn context_windows(catalog: &Value) -> BTreeMap<String, u64> {
    catalog
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let id = item.get("id")?.as_str()?;
            let limit = item
                .get("max_model_len")
                .or_else(|| item.get("context_length"))?
                .as_u64()?;
            (1024..=10_000_000)
                .contains(&limit)
                .then(|| (id.to_owned(), limit))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn text_success_is_not_tool_qualification() {
        use axum::{routing::post, Router};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let app = Router::new().route("/v1/{*path}", post(|| async { r#"{"text":"OK"}"# }));
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let qualified = qualify(&base, None, "local", None).await;
        assert!(qualified
            .protocols
            .iter()
            .all(|check| !check.tool_round_trip));
        assert!(qualified
            .protocols
            .iter()
            .all(|check| check.effort_levels.is_empty()));
        task.abort();
    }
    #[tokio::test]
    #[ignore = "requires an explicitly authorized endpoint and model; sends bounded inference"]
    async fn live_connection_qualification() {
        let base = std::env::var("BUZZ_TEST_MODEL_ENDPOINT").expect("explicit endpoint required");
        let model = std::env::var("BUZZ_TEST_MODEL_ID").expect("explicit model required");
        let (_, limits) = probe::catalog_with_context(&base, None, 5).await.unwrap();
        let qualified = tokio::time::timeout(
            std::time::Duration::from_secs(180),
            qualify(&base, None, &model, limits.get(&model).copied()),
        )
        .await
        .expect("bounded qualification exceeded three minutes");
        println!("{}", serde_json::to_string_pretty(&qualified).unwrap());
        assert!(qualified
            .protocols
            .iter()
            .all(|check| check.tool_round_trip));
    }
    #[tokio::test]
    async fn explicit_qualification_checks_streamed_tools_followup_and_endpoint_effort() {
        qualification_with_followup(true).await;
    }
    #[tokio::test]
    async fn tool_result_without_expected_answer_does_not_qualify() {
        qualification_with_followup(false).await;
    }
    async fn qualification_with_followup(answer_after_tool: bool) {
        use axum::{
            extract::{Path, State},
            routing::post,
            Json, Router,
        };
        use std::sync::{Arc, Mutex};
        let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
        async fn handle(
            State((seen, answer_after_tool)): State<(Arc<Mutex<Vec<Value>>>, bool)>,
            Path(path): Path<String>,
            Json(payload): Json<Value>,
        ) -> (axum::http::StatusCode, String) {
            seen.lock().unwrap().push(payload.clone());
            if let Some(effort) = payload
                .pointer("/output_config/effort")
                .and_then(Value::as_str)
            {
                return if ["low", "medium", "xhigh"].contains(&effort) {
                    (axum::http::StatusCode::OK, "{}".into())
                } else {
                    (
                        axum::http::StatusCode::BAD_REQUEST,
                        "private-server-error".into(),
                    )
                };
            }
            let value = if payload["stream"] == true {
                match path.as_str() {
                    "chat/completions" => format!(
                        "data: {}\n\ndata: [DONE]\n",
                        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call","function":{"name":TOOL,"arguments":json!({"value":NONCE}).to_string()}}]}}]})
                    ),
                    "responses" => format!(
                        "data: {}\n",
                        json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":"call","name":TOOL,"arguments":json!({"value":NONCE}).to_string()}})
                    ),
                    "messages" => format!(
                        "data: {}\n\ndata: {}\n",
                        json!({"type":"content_block_start","content_block":{"type":"tool_use","id":"call","name":TOOL}}),
                        json!({"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":json!({"value":NONCE}).to_string()}})
                    ),
                    _ => panic!("unexpected path"),
                }
            } else {
                assert!(payload.to_string().contains("call"));
                let answer = if answer_after_tool {
                    NONCE
                } else {
                    "unrelated answer"
                };
                match path.as_str() {
                    "chat/completions" => json!({"choices":[{"message":{"content":answer}}]}),
                    "responses" => json!({"output":[{"type":"message","content":[{"type":"output_text","text":answer}]}]}),
                    "messages" => json!({"content":[{"type":"text","text":answer}]}),
                    _ => panic!("unexpected path"),
                }.to_string()
            };
            (axum::http::StatusCode::OK, value)
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/v1/{*path}", post(handle))
            .with_state((seen.clone(), answer_after_tool));
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let qualified = qualify(&base, None, "local", Some(65536)).await;
        assert!(qualified
            .protocols
            .iter()
            .all(|check| check.tool_round_trip == answer_after_tool));
        assert_eq!(
            qualified
                .check("local", Protocol::AnthropicMessages)
                .unwrap()
                .effort_levels,
            if answer_after_tool {
                vec!["low", "medium", "xhigh"]
            } else {
                vec![]
            }
        );
        let requests = seen.lock().unwrap();
        assert_eq!(requests.len(), if answer_after_tool { 11 } else { 6 });
        assert!(requests.iter().all(|request| request["model"] == "local"));
        assert_eq!(qualified.output_token_limit, Some(8192));
        task.abort();
    }
    #[test]
    fn advertised_limit_is_never_inferred_from_a_model_name() {
        let rows = context_windows(
            &json!({"data":[{"id":"opus[1m]"},{"id":"local","max_model_len":65536}]}),
        );
        assert!(!rows.contains_key("opus[1m]"));
        assert_eq!(rows["local"], 65536);
        assert_eq!(output_cap(65536), 8192);
    }
    #[test]
    fn qualification_cannot_cross_model_or_schema_revisions() {
        let q = Qualification {
            revision: REVISION,
            model: "one".into(),
            context_window: None,
            output_token_limit: None,
            protocols: vec![ProtocolCheck {
                protocol: Protocol::AnthropicMessages,
                tool_round_trip: true,
                reason: None,
                effort_levels: vec!["medium".into()],
            }],
        };
        assert_eq!(
            q.check("one", Protocol::AnthropicMessages)
                .unwrap()
                .effort_levels,
            ["medium"]
        );
        assert!(q.check("two", Protocol::AnthropicMessages).is_none());
        assert!(Qualification { revision: 0, ..q }
            .check("one", Protocol::AnthropicMessages)
            .is_none());
    }
    #[test]
    fn streamed_tool_requires_exact_name_arguments_and_call_id() {
        let event = |name: &str, value: &str| {
            format!(
                "data: {}\n\ndata: [DONE]\n",
                json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-1","function":{"name":name,"arguments":json!({"value":value}).to_string()}}]}}]})
            )
        };
        assert!(streamed_tool(event(TOOL, NONCE).as_bytes(), &Protocol::ChatCompletions).is_ok());
        assert!(
            streamed_tool(event("wrong", NONCE).as_bytes(), &Protocol::ChatCompletions).is_err()
        );
        assert!(
            streamed_tool(event(TOOL, "wrong").as_bytes(), &Protocol::ChatCompletions).is_err()
        );
        assert!(streamed_tool(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"OK\"}}]}\n",
            &Protocol::ChatCompletions
        )
        .is_err());
    }
}
