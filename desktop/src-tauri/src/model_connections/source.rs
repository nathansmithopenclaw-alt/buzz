//! Source selection is independent of a harness's optional provider control.
use super::{ModelConnection, CONNECTION_ENV};
use std::collections::BTreeMap;

/// Resolve the saved source before hosted discovery, authentication or launch.
/// An explicit connection wins over ambient provider defaults. The legacy
/// provider marker still requires a connection, rather than falling back.
pub(crate) fn selected<'a>(
    env: &'a BTreeMap<String, String>,
    provider: Option<&str>,
) -> Result<Option<&'a str>, String> {
    if let Some(id) = env.get(CONNECTION_ENV).filter(|id| !id.trim().is_empty()) {
        if id.len() > 256 || id.chars().any(char::is_control) || id.trim() != id {
            return Err("Invalid model connection selection".into());
        }
        return Ok(Some(id));
    }
    if provider.map(str::trim) == Some("buzz-local") {
        return Err("Choose a saved model connection".into());
    }
    Ok(None)
}

/// A connection must never inherit an account's credentials or configuration.
pub(crate) fn validate_account(
    connection: Option<&str>,
    account: Option<&str>,
    inherit_default: bool,
) -> Result<(), String> {
    if connection.is_some() && (account.is_some() || inherit_default) {
        return Err("Choose either a model connection or a subscription account".into());
    }
    Ok(())
}

/// Device-local references cannot be resolved by a remote deployment.
pub(crate) fn validate_remote(connection: Option<&str>) -> Result<(), String> {
    if connection.is_some() {
        return Err("Saved model connections are local to this computer. Run this agent locally or configure its remote host explicitly.".into());
    }
    Ok(())
}

/// Validate against the saved catalog without making a network request. A
/// missing/offline source is never replaced by a similarly named hosted model.
pub(crate) fn model<'a>(row: &ModelConnection, model: Option<&'a str>) -> Result<&'a str, String> {
    let model = model
        .filter(|id| !id.is_empty())
        .ok_or("Choose a model from the saved connection")?;
    if model.len() > 256 || model.chars().any(char::is_control) {
        return Err("Invalid model name".into());
    }
    if !row.models.iter().any(|id| id == model) {
        return Err("This model is not in the saved connection. Refresh the connection.".into());
    }
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_is_authoritative_without_a_provider_control() {
        let env = BTreeMap::from([(CONNECTION_ENV.into(), "source-a".into())]);
        for provider in [None, Some("buzz-local"), Some("openai")] {
            assert_eq!(selected(&env, provider).unwrap(), Some("source-a"));
        }
        assert_eq!(selected(&BTreeMap::new(), Some("openai")).unwrap(), None);
        assert!(selected(&BTreeMap::new(), Some("buzz-local")).is_err());
    }

    #[test]
    fn malformed_selection_cannot_fall_back_to_hosted_auth() {
        for id in [" source-a", "source-a\n"] {
            let env = BTreeMap::from([(CONNECTION_ENV.into(), id.into())]);
            assert!(selected(&env, None).is_err());
        }
    }

    #[test]
    fn saved_source_rejects_account_and_remote_routes() {
        for (account, inherit) in [(Some("account"), false), (None, true)] {
            assert!(validate_account(Some("source-a"), account, inherit).is_err());
            assert!(validate_account(None, account, inherit).is_ok());
        }
        assert!(validate_account(Some("source-a"), None, false).is_ok());
        assert!(validate_remote(Some("source-a")).is_err());
        assert!(validate_remote(None).is_ok());
    }

    #[test]
    fn duplicate_model_names_keep_their_source_identity() {
        let row = |id: &str| ModelConnection {
            id: id.into(),
            name: id.into(),
            base_url: "http://127.0.0.1/v1".into(),
            models: vec!["same-model".into()],
            has_key: false,
            secret_ref: None,
            tested_model: None,
            tested_at: None,
            qualification: None,
        };
        for id in ["source-a", "source-b"] {
            let env = BTreeMap::from([(CONNECTION_ENV.into(), id.into())]);
            assert_eq!(selected(&env, None).unwrap(), Some(id));
            assert_eq!(model(&row(id), Some("same-model")).unwrap(), "same-model");
            assert!(model(&row(id), Some("hosted-default")).is_err());
        }
    }
}
