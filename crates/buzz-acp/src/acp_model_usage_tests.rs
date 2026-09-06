//! Regression for native config changes leaving session/new usage metadata stale.
use super::*;
use serde_json::json;

#[cfg(unix)]
#[tokio::test]
async fn native_config_reply_refreshes_usage_model_only_from_reported_state() {
    let fresh = json!({"configOptions":[
        {"id":"model","category":"model","currentValue":"gpt-5.6-sol"},
        {"id":"reasoning_effort","category":"thought_level","currentValue":"low"}
    ], "models":{"currentModelId":"stale-legacy"}});
    for (reply, expected, success) in [
        (json!({"result":fresh}), "gpt-5.6-sol", true),
        (json!({"result":{}}), "gpt-6-astra[high]", true),
        (
            json!({"error":{"code":-32602,"message":"rejected"}}),
            "gpt-6-astra[high]",
            false,
        ),
    ] {
        let mut wire_reply = reply;
        wire_reply["jsonrpc"] = "2.0".into();
        wire_reply["id"] = 0.into();
        let script = format!("read -r request\nprintf '%s\\n' '{}'\nsleep 5", wire_reply);
        let mut client = AcpClient::spawn("bash", &["-c".into(), script], &[], false)
            .await
            .unwrap();
        client.standard_adapter = Some(StandardAdapterKind::Codex);
        client.record_standard_session_model(
            "target",
            &json!({"models":{"currentModelId":"gpt-6-astra[high]"}}),
        );
        client.record_standard_session_model(
            "sibling",
            &json!({"models":{"currentModelId":"sibling-model"}}),
        );
        let result = client
            .session_set_config_option("target", "model", "requested-but-not-evidence")
            .await;
        assert_eq!(result.is_ok(), success);
        for (session, model) in [("target", expected), ("sibling", "sibling-model")] {
            client.standard_usage.begin_turn(session);
            client
                .parse_prompt_response(
                    session,
                    &json!({
                        "stopReason":"end_turn",
                        "usage":{"inputTokens":10,"outputTokens":2,"totalTokens":12}
                    }),
                )
                .unwrap();
            assert_eq!(
                client.take_turn_usage().unwrap().model.as_deref(),
                Some(model)
            );
        }
        client.shutdown().await;
    }
}
