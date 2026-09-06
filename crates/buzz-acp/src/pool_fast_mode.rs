use super::{AcpClient, AcpError, MODEL_SWITCH_TIMEOUT};

pub(super) async fn apply_fast_mode(
    acp: &mut AcpClient,
    session_id: &str,
    enabled: bool,
    config_options: &serde_json::Value,
) -> Result<serde_json::Value, AcpError> {
    let (config_id, value) = resolve_fast_mode_config_option(config_options, enabled)?;
    let response = tokio::time::timeout(
        MODEL_SWITCH_TIMEOUT,
        acp.session_set_config_option(session_id, config_id, value),
    )
    .await
    .map_err(|_| AcpError::Timeout(MODEL_SWITCH_TIMEOUT))??;
    tracing::info!(
        target: "pool::fast_mode",
        fast_mode = enabled,
        "applied native Fast mode via configOption on session {session_id}"
    );
    acp.observe(
        "fast_mode_selection_verified",
        serde_json::json!({ "enabled": enabled, "configId": config_id }),
    );
    Ok(response
        .get("configOptions")
        .cloned()
        .unwrap_or_else(|| config_options.clone()))
}

pub(super) fn resolve_fast_mode_config_option(
    config_options: &serde_json::Value,
    enabled: bool,
) -> Result<(&str, &'static str), AcpError> {
    let option = config_options.as_array().and_then(|options| {
        options.iter().find(|option| {
            let id = option
                .get("id")
                .or_else(|| option.get("configId"))
                .and_then(|value| value.as_str());
            id == Some("fast")
                || id == Some("fast-mode")
                || (option.get("category").and_then(|value| value.as_str()) == Some("model_config")
                    && option
                        .get("name")
                        .or_else(|| option.get("displayName"))
                        .and_then(|value| value.as_str())
                        .is_some_and(|name| name.eq_ignore_ascii_case("fast mode")))
        })
    });
    let config_id = option
        .and_then(|option| option.get("id").or_else(|| option.get("configId")))
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            AcpError::Protocol(
                "Codex adapter did not advertise a Fast mode control for this session".to_string(),
            )
        })?;
    let desired = if enabled { "on" } else { "off" };
    let advertised = option
        .and_then(|option| option.get("options"))
        .and_then(|value| value.as_array())
        .is_some_and(|options| {
            options
                .iter()
                .any(|option| option.get("value").and_then(|value| value.as_str()) == Some(desired))
        });
    if !advertised {
        return Err(AcpError::Protocol(format!(
            "Fast mode value {desired:?} is not advertised for the selected model"
        )));
    }
    Ok((config_id, desired))
}
