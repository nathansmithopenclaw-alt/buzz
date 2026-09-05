use super::{extract_model_config_options, AcpClient, AcpError, ModelSwitchMethod};
use super::{extract_model_state, model_catalog_value_matches};
use serde_json::Value;

/// Match a desired model ID against a fresh `session/new` response.
///
/// Returns the correct ACP method to call, or `None` if no match.
///
/// **Precedence**: stable `configOptions` first (spec-blessed), then unstable
/// `availableModels`. The fresh `session/new` response is always authoritative.
pub fn resolve_model_switch_method(
    session_new_result: &serde_json::Value,
    desired_model: &str,
) -> Option<ModelSwitchMethod> {
    // 1. Search stable configOptions for a "model"-category entry whose
    //    options contain a value matching desired_model.
    for config_opt in extract_model_config_options(session_new_result) {
        // Adapters disagree on the key: the ACP spec says `configId`, but
        // claude-agent-acp emits `id`. Accept both; the set request always
        // uses `configId` on the wire.
        let config_id = match config_opt
            .get("configId")
            .or_else(|| config_opt.get("id"))
            .and_then(|v| v.as_str())
        {
            Some(id) => id,
            None => continue,
        };
        if let Some(options) = config_opt.get("options").and_then(|v| v.as_array()) {
            for opt in options {
                if let Some(advertised) = opt.get("value").and_then(|v| v.as_str()) {
                    if !model_catalog_value_matches(desired_model, advertised) {
                        continue;
                    }
                    return Some(ModelSwitchMethod::ConfigOption {
                        config_id: config_id.to_string(),
                        option_value: advertised.to_string(),
                    });
                }
            }
        }
    }

    if let Some(method) = resolve_qualified_model(session_new_result, desired_model) {
        return Some(method);
    }

    // 2. Search unstable availableModels for a matching modelId.
    if let Some(models) = extract_model_state(session_new_result) {
        if let Some(available) = models.get("availableModels").and_then(|v| v.as_array()) {
            for model in available {
                if let Some(advertised) = model.get("modelId").and_then(|v| v.as_str()) {
                    if !model_catalog_value_matches(desired_model, advertised) {
                        continue;
                    }
                    return Some(ModelSwitchMethod::SetModel {
                        model_id: advertised.to_string(),
                    });
                }
            }
        }
    }

    // 3. No match.
    None
}

// Codex advertises qualified selectors in legacy models, but separates model
// and effort in the stable controls. The legacy switch returns no fresh controls.
pub(super) fn resolve_qualified_model(
    snapshot: &Value,
    desired: &str,
) -> Option<ModelSwitchMethod> {
    let (model, effort) = desired.strip_suffix(']')?.split_once('[')?;
    if !model.starts_with("gpt-")
        || !matches!(
            effort,
            "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
        )
    {
        return None;
    }
    let advertised = snapshot
        .pointer("/models/availableModels")?
        .as_array()?
        .iter()
        .any(|entry| entry["modelId"].as_str() == Some(desired));
    if !advertised {
        return None;
    }
    for option in extract_model_config_options(snapshot) {
        let matches = option["options"].as_array().is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| entry["value"].as_str() == Some(model))
        });
        if matches {
            return Some(ModelSwitchMethod::ConfigOptionWithEffort {
                config_id: option
                    .get("id")
                    .or_else(|| option.get("configId"))?
                    .as_str()?
                    .into(),
                option_value: model.into(),
                effort: effort.into(),
            });
        }
    }
    None
}

impl AcpClient {
    /// Apply a model-switch method resolved from the adapter's current catalog.
    pub async fn apply_model_switch_method(
        &mut self,
        session_id: &str,
        method: &ModelSwitchMethod,
    ) -> Result<Value, AcpError> {
        match method {
            ModelSwitchMethod::ConfigOption {
                config_id,
                option_value,
            } => {
                self.session_set_config_option(session_id, config_id, option_value)
                    .await
            }
            ModelSwitchMethod::ConfigOptionWithEffort {
                config_id,
                option_value,
                effort,
            } => {
                self.session_set_model_with_effort(session_id, config_id, option_value, effort)
                    .await
            }
            ModelSwitchMethod::SetModel { model_id } => {
                self.session_set_model(session_id, model_id).await
            }
        }
    }

    /// Select a Codex model and its explicit effort using fresh stable controls.
    pub async fn session_set_model_with_effort(
        &mut self,
        session_id: &str,
        config_id: &str,
        model: &str,
        effort: &str,
    ) -> Result<Value, AcpError> {
        let selected = self
            .session_set_config_option(session_id, config_id, model)
            .await?;
        let options = selected["configOptions"].as_array().ok_or_else(|| {
            AcpError::Protocol("Model selection returned no configuration controls".into())
        })?;
        let accepted = options.iter().find(|o| {
            o.get("id")
                .or_else(|| o.get("configId"))
                .and_then(Value::as_str)
                == Some(config_id)
        });
        if accepted.and_then(|o| o["currentValue"].as_str()) != Some(model) {
            return Err(AcpError::Protocol(
                "Model selection did not confirm the requested model".into(),
            ));
        }
        let control = options
            .iter()
            .find(|o| o["category"].as_str() == Some("thought_level"))
            .filter(|o| {
                o["options"].as_array().is_some_and(|values| {
                    values.iter().any(|v| v["value"].as_str() == Some(effort))
                })
            })
            .ok_or_else(|| {
                AcpError::Protocol(format!(
                    "Selected model does not advertise requested effort {effort:?}"
                ))
            })?;
        let effort_id = control
            .get("id")
            .or_else(|| control.get("configId"))
            .and_then(Value::as_str)
            .ok_or_else(|| AcpError::Protocol("Reasoning effort control has no ID".into()))?;
        let updated = self
            .session_set_config_option(session_id, effort_id, effort)
            .await?;
        let confirmed = updated["configOptions"].as_array().and_then(|opts| {
            opts.iter().find(|o| {
                o.get("id")
                    .or_else(|| o.get("configId"))
                    .and_then(Value::as_str)
                    == Some(effort_id)
            })
        });
        if confirmed.and_then(|o| o["currentValue"].as_str()) != Some(effort) {
            return Err(AcpError::Protocol(
                "Reasoning effort selection was not confirmed".into(),
            ));
        }
        Ok(updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualified_model_requires_both_catalogs_and_known_effort() {
        let snapshot = serde_json::json!({
            "configOptions": [{"id":"model","category":"model","options":[{"value":"gpt-5.6-sol"}]}],
            "models": {"availableModels":[{"modelId":"gpt-5.6-sol[medium]"}, {"modelId":"gpt-5.6-sol[1m]"}]}
        });
        assert!(
            matches!(resolve_qualified_model(&snapshot, "gpt-5.6-sol[medium]"), Some(ModelSwitchMethod::ConfigOptionWithEffort { effort, .. }) if effort == "medium")
        );
        for desired in [
            "gpt-5.6-sol",
            "gpt-5.6-sol[high]",
            "gpt-5.6-sol[1m]",
            "gpt-5.6-sol[medium][high]",
            "opus[medium]",
        ] {
            assert!(resolve_qualified_model(&snapshot, desired).is_none());
        }
        let mut missing = snapshot.clone();
        missing["configOptions"] = serde_json::json!([]);
        assert!(resolve_qualified_model(&missing, "gpt-5.6-sol[medium]").is_none());
        missing = snapshot;
        missing["models"] = serde_json::Value::Null;
        assert!(resolve_qualified_model(&missing, "gpt-5.6-sol[medium]").is_none());
    }

    #[tokio::test]
    async fn stable_model_effort_rejects_unconfirmed_or_unadvertised_selection() {
        let valid = serde_json::json!({"configOptions":[
            {"id":"model","category":"model","currentValue":"gpt-5.6-sol"},
            {"id":"reasoning_effort","category":"thought_level","currentValue":"medium","options":[{"value":"medium"}]}
        ]});
        let mut wrong_model = valid.clone();
        wrong_model["configOptions"][0]["currentValue"] = "gpt-6-astra".into();
        let mut missing_effort = valid.clone();
        missing_effort["configOptions"][1]["options"] = serde_json::json!([]);
        let mut wrong_effort = valid.clone();
        wrong_effort["configOptions"][1]["currentValue"] = "low".into();
        for (first, second) in [
            (serde_json::json!({}), valid.clone()),
            (wrong_model, valid.clone()),
            (missing_effort, valid.clone()),
            (valid.clone(), wrong_effort),
            (valid, serde_json::json!({})),
        ] {
            let script = format!("import json,sys\nreplies={}\nfor i,line in enumerate(sys.stdin):\n r=json.loads(line); print(json.dumps(dict(jsonrpc='2.0',id=r['id'],result=replies[min(i,1)])),flush=True)\n", serde_json::json!([first, second]));
            let mut acp =
                AcpClient::spawn("python3", &["-u".into(), "-c".into(), script], &[], false)
                    .await
                    .unwrap();
            assert!(acp
                .session_set_model_with_effort("session", "model", "gpt-5.6-sol", "medium")
                .await
                .is_err());
            acp.shutdown().await;
        }
    }
}
