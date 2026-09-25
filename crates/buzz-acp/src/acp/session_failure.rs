//! Negotiated JetBrains AIR v1 terminal session failures.
//!
//! Session-info updates can describe retries, background turns, or cleared
//! notices. Only the response to our prompt is authoritative for this turn.

use super::AcpError;
use serde_json::Value;

const AIR: &str = "/_meta/jetbrains/air";

pub(super) fn advertised(result: &Value) -> bool {
    let Some(air) = result.pointer(AIR) else {
        return false;
    };
    air["version"].as_u64() == Some(1)
        && air["capabilities"].as_array().is_some_and(|items| {
            items
                .iter()
                .any(|item| item.as_str() == Some("sessionFailure"))
        })
}

pub(super) fn terminal_error(result: &Value) -> Result<bool, AcpError> {
    let Some(air) = result.pointer(AIR) else {
        return Ok(false);
    };
    let Some(failure) = air.get("sessionFailure").filter(|value| !value.is_null()) else {
        return Ok(false);
    };
    // Validate the v1 envelope, but never retain or echo its untrusted title,
    // details, provider body, or actions into the channel-facing notice.
    let valid = air["version"].as_u64() == Some(1)
        && failure.is_object()
        && ["id", "category", "title"]
            .iter()
            .all(|field| failure[field].as_str().is_some())
        && failure["revision"].as_u64().is_some()
        && failure["actions"]
            .as_array()
            .is_some_and(|actions| actions.iter().all(Value::is_string));
    if valid {
        match failure["severity"].as_str() {
            Some("error") => return Ok(true),
            Some("warning" | "info") => return Ok(false),
            _ => {}
        }
    }
    Err(AcpError::Protocol(
        "Malformed negotiated AIR v1 sessionFailure metadata".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response() -> Value {
        json!({"stopReason":"end_turn", "usage":{"inputTokens":10,"outputTokens":2,"totalTokens":12},
        "_meta":{"jetbrains":{"air":{"version":1,"sessionFailure":{
            "id":"turn:error","revision":1,"category":"service","severity":"error",
            "title":"untrusted provider body", "actions":["retry"]
        }}}}})
    }

    #[test]
    fn malformed_failure_fields_never_become_success_or_leak_payloads() {
        for (field, value) in [
            ("id", Value::Null),
            ("revision", json!(-1)),
            ("category", json!({})),
            ("severity", json!("unrecognized")),
            ("title", json!(42)),
            ("actions", json!([42])),
        ] {
            let mut result = response();
            result["_meta"]["jetbrains"]["air"]["sessionFailure"][field] = value;
            let error = terminal_error(&result).unwrap_err().to_string();
            assert!(error.contains("Malformed negotiated"), "{field}");
            assert!(!error.contains("untrusted provider body"));
        }
        let mut result = response();
        result["_meta"]["jetbrains"]["air"]["version"] = json!(2);
        assert!(terminal_error(&result).is_err());
    }

    #[tokio::test]
    async fn decoder_preserves_usage_and_private_capture_rejects_failure() {
        use super::super::{AcpClient, StandardAdapterKind, StopReason};
        let mut client =
            AcpClient::spawn("bash", &["-c".into(), "read -r request".into()], &[], false)
                .await
                .unwrap();
        client.standard_adapter = Some(StandardAdapterKind::Codex);
        client.record_standard_session_model(
            "session",
            &json!({"models":{"currentModelId":"model"}}),
        );
        client.standard_usage.begin_turn("session");
        // Unadvertised extension is ignored, even if a peer emits it anyway.
        assert_eq!(
            client
                .parse_prompt_response("session", &response())
                .unwrap(),
            StopReason::EndTurn
        );
        assert!(
            !client
                .turn_delivery_evidence()
                .adapter_runtime_failure_present
        );
        client.typed_session_failure_supported = true;
        client
            .parse_prompt_response("session", &response())
            .unwrap();
        assert!(
            client
                .turn_delivery_evidence()
                .adapter_runtime_failure_present
        );
        assert!(client.take_turn_usage().is_some());
        client.turn_delivery_evidence = Default::default();
        client.private_capture = Some(String::new());
        assert!(matches!(
            client.parse_prompt_response("session", &response()),
            Err(AcpError::AgentRuntimeFailure)
        ));
        assert!(
            !client
                .turn_delivery_evidence()
                .adapter_runtime_failure_present
        );
        client.shutdown().await;
    }

    #[test]
    fn advertisement_requires_supported_version_and_capability() {
        for (air, expected) in [
            (json!({"version":1,"capabilities":["sessionFailure"]}), true),
            (
                json!({"version":2,"capabilities":["sessionFailure"]}),
                false,
            ),
            (
                json!({"version":"1","capabilities":["sessionFailure"]}),
                false,
            ),
            (json!({"version":1,"capabilities":[]}), false),
            (json!({"version":1,"capabilities":"sessionFailure"}), false),
            (Value::Null, false),
        ] {
            assert_eq!(
                advertised(&json!({"_meta":{"jetbrains":{"air":air}}})),
                expected
            );
        }
    }
}
