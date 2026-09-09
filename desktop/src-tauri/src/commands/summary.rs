//! Scoped, authenticated control requests for named Summaries.
use crate::{
    app_state::AppState,
    relay::{
        assert_expected_relay_scope, assert_expected_signer, build_nip98_auth_header_for_keys,
        classify_request_error, parse_json_response, relay_api_base_url_with_override,
        relay_error_message,
    },
};
use serde_json::Value;

#[tauri::command]
pub async fn summary_operation(
    state: tauri::State<'_, AppState>,
    request: Value,
    expected_relay_url: String,
    expected_signer_pubkey: String,
) -> Result<Value, String> {
    let base = relay_api_base_url_with_override(&state);
    let keys = state.signing_keys()?;
    assert_expected_relay_scope(Some(&expected_relay_url), &base)?;
    assert_expected_signer(Some(&expected_signer_pubkey), &keys.public_key().to_hex())?;
    let operation = request
        .get("summary_operation")
        .and_then(Value::as_str)
        .ok_or("Missing Summary operation")?;
    validate_summary_operation(operation)?;
    let url = format!("{}/query", base);
    let body = serde_json::to_vec(&[request]).map_err(|e| e.to_string())?;
    let auth = build_nip98_auth_header_for_keys(&keys, &reqwest::Method::POST, &url, &body)?;
    let response = state
        .http_client
        .post(&url)
        .header("Authorization", auth)
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(30))
        .body(body)
        .send()
        .await
        .map_err(|e| classify_request_error(&e))?;
    if !response.status().is_success() {
        return Err(relay_error_message(response).await);
    }
    let result = parse_json_response(response).await?;
    assert_expected_relay_scope(
        Some(&expected_relay_url),
        &relay_api_base_url_with_override(&state),
    )?;
    assert_expected_signer(
        Some(&expected_signer_pubkey),
        &state.signing_keys()?.public_key().to_hex(),
    )?;
    Ok(result)
}

fn validate_summary_operation(operation: &str) -> Result<(), String> {
    if ![
        "definitions",
        "overview",
        "dispatch",
        "reports_get",
        "reports_list",
        "reports_archive",
        "reports_restore",
        "reports_delete",
        "people",
        "start",
        "status",
        "cursors",
        "history",
        "capabilities",
        "progress",
    ]
    .contains(&operation)
    {
        return Err("Unknown Summary operation".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_summary_operation;

    #[test]
    fn summary_bridge_accepts_frontend_requests() {
        // Bind the real callers to the production bridge validation so browser
        // fixtures cannot conceal an operation rejected by the native shell.
        let callers = [
            include_str!("../../../src/features/pulse/lib/namedSummaries.ts"),
            include_str!("../../../src/features/pulse/lib/runSummary.ts"),
            include_str!("../../../src/features/pulse/lib/summaryReports.ts"),
            include_str!("../../../src/features/pulse/ui/SummariesView.tsx"),
            include_str!("../../../src/features/pulse/ui/SummaryAudienceFields.tsx"),
            include_str!("../../../src/features/pulse/ui/SummaryRunHistory.tsx"),
        ];
        let mut checked = 0;
        for source in callers {
            for suffix in source.split("summary_operation: \"").skip(1) {
                let operation = suffix.split('"').next().unwrap();
                assert!(validate_summary_operation(operation).is_ok(), "{operation}");
                checked += 1;
            }
        }
        assert!(checked >= 10);
        for operation in ["reports_archive", "reports_restore", "reports_delete"] {
            assert!(validate_summary_operation(operation).is_ok());
        }
        for operation in ["", "publish", "admin", "reports_unknown"] {
            assert!(validate_summary_operation(operation).is_err());
        }
    }
}

#[tauri::command]
pub async fn publish_summary_definition(
    state: tauri::State<'_, AppState>,
    event: String,
    expected_relay_url: String,
    expected_signer_pubkey: String,
) -> Result<crate::relay::SubmitEventResponse, String> {
    let base = relay_api_base_url_with_override(&state);
    let keys = state.signing_keys()?;
    assert_expected_relay_scope(Some(&expected_relay_url), &base)?;
    assert_expected_signer(Some(&expected_signer_pubkey), &keys.public_key().to_hex())?;
    let event: nostr::Event = serde_json::from_str(&event).map_err(|e| e.to_string())?;
    if event.kind.as_u16() != 30623 {
        return Err("Expected a Summary definition".into());
    }
    crate::relay::submit_signed_event_at_with_keys(&event, &state, &base, &keys).await
}
