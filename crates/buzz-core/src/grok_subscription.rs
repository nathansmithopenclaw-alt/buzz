//! Shared, credential-free policy for selected Grok subscription accounts.
use serde_json::Value;

/// Provider environment overrides that must not outrank a selected profile.
pub fn competing_env(name: &str) -> bool {
    name.starts_with("XAI_") || name.starts_with("GROK_")
}

/// Reject credential, endpoint and executable authentication overrides at any depth.
pub fn validate_config(value: &Value) -> Result<(), String> {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let key = key.to_ascii_lowercase();
                if [
                    "api_key",
                    "env_key",
                    "base_url",
                    "endpoints",
                    "oidc",
                    "issuer",
                    "client_id",
                    "auth_provider_command",
                    "auth_provider",
                    "bearer_token",
                    "access_token",
                    "refresh_token",
                    "token",
                    "api_key_env",
                    "headers",
                    "alpha_test_key",
                    "proxy_url",
                    "deployment_key",
                ]
                .contains(&key.as_str())
                    || key.contains("base_url")
                    || key.contains("api_key") && key != "disable_api_key_auth"
                    || key.starts_with("auth_provider")
                    || key.starts_with("oidc_")
                    || (key != "grok_com_config" && competing_env(&key.to_ascii_uppercase()))
                    || (key == "preferred_method" && value.as_str() != Some("oidc"))
                    || (key == "disable_api_key_auth" && value.as_bool() != Some(true))
                {
                    return Err("Grok configuration contains competing authentication; remove the override before selecting a subscription account".into());
                }
                validate_config(value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_config(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Validate provider metadata; this does not establish billing attribution.
pub fn verify_status(info: &Value, status: &Value) -> Result<(), String> {
    let meta = &status["meta"];
    let tier = meta["subscription_tier"].as_str().unwrap_or("").trim();
    if status["authenticated"].as_bool() != Some(true)
        || meta["auth_mode"].as_str() != Some("Oidc")
        || !matches!(
            info["methodId"].as_str(),
            Some("cached_token" | "grok.com" | "oidc")
        )
        || info["email"].as_str().is_none_or(|s| s.trim().is_empty())
        || !info["userBlockedReason"].is_null()
        || info["teamBlockedReasons"]
            .as_array()
            .is_none_or(|a| !a.is_empty())
        || !meta.as_object().is_some_and(|m| m.contains_key("gate"))
        || !meta["gate"].is_null()
        || tier.is_empty()
        || tier.eq_ignore_ascii_case("free")
    {
        return Err("Grok did not confirm subscription authentication and access; reconnect with an eligible subscription account".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn rejects_nested_payer_overrides() {
        for key in [
            "api_key",
            "env_key",
            "base_url",
            "oidc",
            "auth_provider_command",
            "XAI_API_KEY",
        ] {
            assert!(validate_config(&json!({"model":{"test":{key:"override"}}})).is_err());
        }
        assert!(
            validate_config(&json!({"models":{"default":"grok-4.6"},"mcp_servers":{}})).is_ok()
        );
    }
    #[test]
    fn status_requires_subscription_identity_and_access() {
        let info = json!({"methodId":"cached_token","email":"test@example.invalid","userBlockedReason":null,"teamBlockedReasons":[]});
        let status = json!({"authenticated":true,"meta":{"auth_mode":"Oidc","subscription_tier":"SuperGrok Heavy","gate":null}});
        assert!(verify_status(&info, &status).is_ok());
        for (key, value) in [
            ("auth_mode", json!("ApiKey")),
            ("subscription_tier", json!("Free")),
            ("gate", json!({"message":"Upgrade"})),
        ] {
            let mut bad = status.clone();
            bad["meta"][key] = value;
            assert!(verify_status(&info, &bad).is_err());
        }
        assert!(verify_status(&info, &json!({"authenticated":false,"meta":null})).is_err());
        let mut missing = status.clone();
        missing["meta"].as_object_mut().unwrap().remove("gate");
        assert!(verify_status(&info, &missing).is_err());
    }
}
