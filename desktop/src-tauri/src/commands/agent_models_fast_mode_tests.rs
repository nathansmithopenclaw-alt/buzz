use super::*;

#[test]
fn carries_native_fast_mode_state() {
    for (id, current_value, expected) in [
        ("fast", serde_json::json!("on"), true),
        ("fast-mode", serde_json::json!(false), false),
    ] {
        let response = normalize_agent_models(
            &serde_json::json!({
                "stable": {
                    "configOptions": [{
                        "id": id,
                        "name": "Fast mode",
                        "description": "Faster responses on supported models",
                        "category": "model_config",
                        "currentValue": current_value,
                        "options": [{ "value": "on" }, { "value": "off" }]
                    }]
                }
            }),
            None,
        );

        assert_eq!(response.fast_mode_config_id.as_deref(), Some(id));
        assert_eq!(response.fast_mode_enabled, Some(expected));
        assert_eq!(
            response.fast_mode_description.as_deref(),
            Some("Faster responses on supported models")
        );
    }
}

#[test]
fn omits_fast_mode_when_model_does_not_advertise_it() {
    let response = normalize_agent_models(
        &serde_json::json!({ "stable": { "configOptions": [] } }),
        None,
    );

    assert_eq!(response.fast_mode_config_id, None);
    assert_eq!(response.fast_mode_enabled, None);
}
