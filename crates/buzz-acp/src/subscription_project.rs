use super::{validate_subscription_codex_config, AcpError};

pub(super) fn validate_subscription_project_context(
    harness: &str,
    cwd: &std::path::Path,
) -> Result<(), AcpError> {
    for parent in cwd.ancestors() {
        let names: &[&str] = if harness == "claude" {
            &[".claude/settings.json", ".claude/settings.local.json"]
        } else {
            &[".codex/config.toml"]
        };
        for name in names {
            let path = parent.join(name);
            let meta = match std::fs::metadata(&path) {
                Ok(meta) => meta,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    return Err(AcpError::Protocol(format!(
                        "Project configuration unavailable: {e}"
                    )))
                }
            };
            if !meta.is_file() || meta.len() > 1024 * 1024 {
                return Err(AcpError::Protocol(
                    "Project configuration is not a bounded regular file".into(),
                ));
            }
            let raw = std::fs::read_to_string(path)?;
            let value: serde_json::Value = if harness == "claude" {
                serde_json::from_str(&raw)
                    .map_err(|_| AcpError::Protocol("Invalid Claude project settings".into()))?
            } else {
                let value: toml::Table = toml::from_str(&raw)
                    .map_err(|_| AcpError::Protocol("Invalid Codex project settings".into()))?;
                serde_json::to_value(value).map_err(|e| AcpError::Protocol(e.to_string()))?
            };
            validate_subscription_codex_config(&value)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_project_accepts_valid_toml_documents() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".codex")).unwrap();
        let nested = root.path().join("project/subdir");
        std::fs::create_dir_all(&nested).unwrap();
        for raw in [
            "",
            "model = 'fixture-model'\n",
            "[mcp_servers.computer-use]\ncommand = '/bin/true'\ncwd = '.'\n",
        ] {
            std::fs::write(root.path().join(".codex/config.toml"), raw).unwrap();
            validate_subscription_project_context("codex", &nested).unwrap();
        }
    }

    #[test]
    fn codex_project_rejects_malformed_documents_and_auth_overrides() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".codex")).unwrap();
        for (raw, expected) in [
            ("[broken", "Invalid Codex project settings"),
            (
                "api_key = 'fixture-key'",
                "conflicts with subscription authentication",
            ),
            (
                "model_provider = 'other'",
                "conflicts with subscription authentication",
            ),
            (
                "[mcp_servers.example.env]\nOPENAI_API_KEY = 'fixture-key'",
                "conflicts with subscription authentication",
            ),
        ] {
            std::fs::write(root.path().join(".codex/config.toml"), raw).unwrap();
            let error = validate_subscription_project_context("codex", root.path()).unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn selected_codex_session_reaches_adapter_with_project_config() {
        use super::super::AcpClient;
        use std::os::unix::fs::PermissionsExt;
        const MARKER: &str = "BUZZ_TEST_CODEX_PROJECT_SESSION";
        if std::env::var_os(MARKER).is_none() {
            let root = tempfile::tempdir().unwrap();
            let adapter = root.path().join("codex-acp");
            std::fs::write(&adapter, r#"#!/usr/bin/python3
import json, sys
for line in sys.stdin:
    req = json.loads(line)
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':{'sessionId':'project-config-accepted'}}), flush=True)
"#).unwrap();
            std::fs::set_permissions(&adapter, std::fs::Permissions::from_mode(0o700)).unwrap();
            std::fs::create_dir(root.path().join(".codex")).unwrap();
            std::fs::write(root.path().join(".codex/config.toml"), "model = 'fixture-model'\n[mcp_servers.computer-use]\ncommand = '/bin/true'\ncwd = '.'\n").unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env(MARKER, &adapter)
                .env("BUZZ_ACP_SUBSCRIPTION_ACCOUNT_ID", "fixture-account")
                .env("BUZZ_ACP_SUBSCRIPTION_HARNESS", "codex")
                .env("BUZZ_ACP_SUBSCRIPTION_HOME", root.path())
                .args(["--exact", "acp::subscription_project::tests::selected_codex_session_reaches_adapter_with_project_config", "--nocapture"])
                .output().unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let adapter = std::env::var(MARKER).unwrap();
        let cwd = std::env::var("BUZZ_ACP_SUBSCRIPTION_HOME").unwrap();
        let mut client = AcpClient::spawn(&adapter, &[], &[], false).await.unwrap();
        let response = client
            .session_new_full(&cwd, vec![], None, None)
            .await
            .unwrap();
        assert_eq!(response.session_id, "project-config-accepted");
        std::fs::write(
            std::path::Path::new(&cwd).join(".codex/config.toml"),
            "model_provider = 'other'",
        )
        .unwrap();
        let error = client
            .session_new_full(&cwd, vec![], None, None)
            .await
            .err()
            .expect("authentication override must be rejected");
        assert!(error
            .to_string()
            .contains("conflicts with subscription authentication"));
        client.shutdown().await;
    }
}
