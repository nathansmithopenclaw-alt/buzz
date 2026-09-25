//! Optional provider routing through the adapter's negotiated ACP extension.
//! Credentials enter the broker through env and leave only through ACP stdin.
use super::{AcpClient, AcpError};
use serde::Deserialize;
use std::collections::BTreeMap;

pub(super) const PROVIDER_CONFIG_ENV: &str = "BUZZ_ACP_PROVIDER_CONFIG";

// Never derive Debug: this structure owns authentication headers.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ProviderConfig {
    provider_id: String,
    api_type: String,
    base_url: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

impl ProviderConfig {
    pub(super) fn from_env(extra: &[(String, String)]) -> Result<Option<Self>, AcpError> {
        let raw = extra
            .iter()
            .rev()
            .find(|(key, _)| key == PROVIDER_CONFIG_ENV)
            .map(|(_, value)| value.clone())
            .or_else(|| std::env::var(PROVIDER_CONFIG_ENV).ok());
        raw.as_deref()
            .map(Self::parse)
            .transpose()
            .map(Option::flatten)
    }

    fn parse(raw: &str) -> Result<Option<Self>, AcpError> {
        let invalid = || AcpError::Protocol("Invalid ACP provider configuration".into());
        if raw.trim().is_empty() {
            return Ok(None);
        }
        if raw.len() > 64 * 1024 {
            return Err(invalid());
        }
        let config: Self = serde_json::from_str(raw).map_err(|_| invalid())?;
        let identifier = |value: &str| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        };
        let url = url::Url::parse(&config.base_url).map_err(|_| invalid())?;
        if !identifier(&config.provider_id)
            || !identifier(&config.api_type)
            || !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || config.headers.len() > 32
            || config.headers.iter().any(|(name, value)| {
                !identifier(name) || value.len() > 8192 || value.chars().any(char::is_control)
            })
        {
            return Err(invalid());
        }
        Ok(Some(config))
    }
}

impl AcpClient {
    pub(super) async fn configure_provider(
        &mut self,
        init: &serde_json::Value,
    ) -> Result<(), AcpError> {
        let Some(config) = self.provider_config.clone() else {
            return Ok(());
        };
        if !init
            .pointer("/agentCapabilities/providers")
            .is_some_and(serde_json::Value::is_object)
        {
            return Err(AcpError::Protocol(
                "This adapter does not support ACP provider configuration".into(),
            ));
        }
        let providers = self
            .private_provider_request("providers/list", serde_json::json!({}))
            .await?;
        let supported = providers
            .get("providers")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|rows| {
                rows.iter().any(|row| {
                    row.get("providerId").and_then(serde_json::Value::as_str)
                        == Some(&config.provider_id)
                        && row
                            .get("supported")
                            .and_then(serde_json::Value::as_array)
                            .is_some_and(|apis| {
                                apis.iter()
                                    .any(|api| api.as_str() == Some(&config.api_type))
                            })
                })
            });
        if !supported {
            return Err(AcpError::Protocol(
                "This adapter does not support the selected provider protocol".into(),
            ));
        }
        self.private_provider_request(
            "providers/set",
            serde_json::json!({
                "providerId": config.provider_id, "apiType": config.api_type,
                "baseUrl": config.base_url, "headers": config.headers,
            }),
        )
        .await?;
        Ok(())
    }

    async fn private_provider_request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, AcpError> {
        // Both directions are private: providers/list and even error replies
        // may echo credentials. Retain the ordinary bounded request transport.
        self.private_provider_rpc = true;
        let result = self.send_request(method, params).await;
        self.private_provider_rpc = false;
        result.map_err(|_| {
            AcpError::Protocol(format!(
                "ACP {method} failed; check the selected provider configuration"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct TraceBuffer(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for TraceBuffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn config() -> String {
        serde_json::json!({"providerId":"test-provider","apiType":"openai",
            "baseUrl":"http://127.0.0.1:8010/v1","headers":{"Authorization":"Bearer synthetic-provider-secret"}}).to_string()
    }

    #[test]
    fn invalid_configuration_never_echoes_credential_material() {
        for raw in [
            "synthetic-provider-secret".to_string(),
            config().replace(
                "http://127.0.0.1:8010/v1",
                "https://synthetic-provider-secret@host/v1",
            ),
            config().replace("Bearer synthetic-provider-secret", "bad\\nheader"),
        ] {
            let error = ProviderConfig::parse(&raw).err().unwrap().to_string();
            assert!(!error.contains("synthetic-provider-secret"));
        }
        assert!(ProviderConfig::parse(&config()).unwrap().is_some());
        assert!(ProviderConfig::parse("").unwrap().is_none());
    }

    #[tokio::test]
    async fn provider_is_negotiated_privately_before_session_creation() {
        let trace = TraceBuffer::default();
        let writer = trace.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        tracing::info!("provider-test-trace-control");
        let script = r#"
import json,sys,os
assert 'BUZZ_ACP_PROVIDER_CONFIG' not in os.environ
expected=['initialize','providers/list','providers/set','session/new']
for method in expected:
    request=json.loads(sys.stdin.readline())
    assert request['method']==method,request['method']
    if method=='initialize': result={'protocolVersion':1,'agentCapabilities':{'providers':{}}}
    elif method=='providers/list': result={'providers':[{'providerId':'test-provider','supported':['openai'],'current':{'headers':{'Authorization':'synthetic-provider-secret'}}}]}
    elif method=='providers/set':
        assert request['params']['headers']['Authorization']=='Bearer synthetic-provider-secret'
        print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'unexpected','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'synthetic-provider-secret'}}}}),flush=True)
        result={'echo':'synthetic-provider-secret'}
    else: result={'sessionId':'configured-session'}
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}),flush=True)
"#;
        let mut client = AcpClient::spawn(
            "python3",
            &["-c".into(), script.into()],
            &[(PROVIDER_CONFIG_ENV.into(), config())],
            false,
        )
        .await
        .unwrap();
        let observer = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(observer.clone()), 0);
        client.initialize().await.unwrap();
        let session = client
            .session_new_full("/tmp", vec![], None, None)
            .await
            .unwrap();
        assert_eq!(session.session_id, "configured-session");
        let observed = serde_json::to_string(&observer.snapshot()).unwrap();
        assert!(!observed.contains("synthetic-provider-secret"));
        assert!(observed.contains("session/new"));
        let traced = String::from_utf8(trace.0.lock().unwrap().clone()).unwrap();
        assert!(!traced.contains("synthetic-provider-secret"));
        assert!(traced.contains("provider-test-trace-control"));
    }

    #[tokio::test]
    async fn provider_error_is_redacted_and_cannot_create_a_session() {
        let script = r#"
import json,sys
request=json.loads(sys.stdin.readline())
print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':{'protocolVersion':1,'agentCapabilities':{'providers':{}}}}),flush=True)
request=json.loads(sys.stdin.readline())
assert request['method']=='providers/list'
print(json.dumps({'jsonrpc':'2.0','id':request['id'],'error':{'code':-32603,'message':'synthetic-provider-secret'}}),flush=True)
"#;
        let mut client = AcpClient::spawn(
            "python3",
            &["-c".into(), script.into()],
            &[(PROVIDER_CONFIG_ENV.into(), config())],
            false,
        )
        .await
        .unwrap();
        let error = client.initialize().await.unwrap_err().to_string();
        assert!(error.contains("ACP providers/list failed"));
        assert!(!error.contains("synthetic-provider-secret"));
    }

    #[tokio::test]
    async fn missing_provider_capability_fails_before_any_session() {
        let script = "read line; echo '{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"protocolVersion\":1,\"agentCapabilities\":{}}}'; sleep 1";
        let mut client = AcpClient::spawn(
            "bash",
            &["-c".into(), script.into()],
            &[(PROVIDER_CONFIG_ENV.into(), config())],
            false,
        )
        .await
        .unwrap();
        let error = client.initialize().await.unwrap_err().to_string();
        assert!(error.contains("does not support ACP provider configuration"));
    }
}
