//! Revalidate the selected Grok profile at the actual adapter/session boundary.
use super::AcpError;
use buzz_core::grok_subscription::{competing_env, validate_config};
use std::path::Path;

fn config(path: &Path, required: bool) -> Result<(), AcpError> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if !required && e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            return Err(AcpError::Protocol(
                "Grok subscription settings unavailable".into(),
            ))
        }
    };
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 1024 * 1024 {
        return Err(AcpError::Protocol(
            "Invalid Grok subscription settings file".into(),
        ));
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|_| AcpError::Protocol("Could not read Grok settings".into()))?;
    let value: toml::Value =
        toml::from_str(&raw).map_err(|_| AcpError::Protocol("Invalid Grok settings".into()))?;
    let value = serde_json::to_value(value)
        .map_err(|_| AcpError::Protocol("Invalid Grok settings".into()))?;
    validate_config(&value).map_err(AcpError::Protocol)?;
    if required
        && (value["auth"]["preferred_method"].as_str() != Some("oidc")
            || value["grok_com_config"]["disable_api_key_auth"].as_bool() != Some(true))
    {
        return Err(AcpError::Protocol(
            "Grok subscription authentication policy is missing".into(),
        ));
    }
    Ok(())
}

pub(super) fn project(cwd: &Path) -> Result<(), AcpError> {
    for parent in cwd.ancestors() {
        config(&parent.join(".grok/config.toml"), false)?;
    }
    for name in ["config.toml", "managed_config.toml", "requirements.toml"] {
        config(&Path::new("/etc/grok").join(name), false)?;
    }
    Ok(())
}

pub(super) fn configure(child: &mut tokio::process::Command, home: &Path) -> Result<(), AcpError> {
    if !home.is_absolute() {
        return Err(AcpError::Protocol(
            "Grok account home must be absolute".into(),
        ));
    }
    config(&home.join("config.toml"), true)?;
    config(&home.join("requirements.toml"), true)?;
    config(&home.join("managed_config.toml"), false)?;
    let names: Vec<_> = std::env::vars_os()
        .map(|(k, _)| k)
        .chain(child.as_std().get_envs().map(|(k, _)| k.to_owned()))
        .collect();
    for name in names {
        if competing_env(&name.to_string_lossy()) {
            child.env_remove(name);
        }
    }
    child.env("GROK_HOME", home);
    child.arg("--leader-socket").arg(home.join("leader.sock"));
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    #[tokio::test]
    async fn selected_grok_reaches_actual_spawn_with_isolated_profile() {
        const MARKER: &str = "BUZZ_TEST_GROK_SELECTED_SPAWN";
        if std::env::var_os(MARKER).is_none() {
            let root = tempfile::tempdir().unwrap();
            let policy =
                "[auth]\npreferred_method='oidc'\n[grok_com_config]\ndisable_api_key_auth=true\n";
            for name in ["config.toml", "requirements.toml"] {
                std::fs::write(root.path().join(name), policy).unwrap();
            }
            let executable = root.path().join("grok");
            std::fs::write(&executable,r#"#!/usr/bin/python3
import json,os,sys
for line in sys.stdin:
 r=json.loads(line)
 print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':{'sessionId':'fixture','home':os.environ.get('GROK_HOME'),'key':os.environ.get('XAI_API_KEY'),'proxy':os.environ.get('GROK_CLI_CHAT_PROXY_BASE_URL'),'args':sys.argv[1:]}}),flush=True)
"#).unwrap();
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
            let out=std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact","acp::subscription_grok::tests::selected_grok_reaches_actual_spawn_with_isolated_profile","--nocapture"])
                .env(MARKER,&executable).env("BUZZ_ACP_SUBSCRIPTION_ACCOUNT_ID","fixture")
                .env("BUZZ_ACP_SUBSCRIPTION_HARNESS","grok").env("BUZZ_ACP_SUBSCRIPTION_KIND","login")
                .env("BUZZ_ACP_SUBSCRIPTION_HOME",root.path()).env("XAI_API_KEY","synthetic-inherited")
                .env("GROK_HOME","/wrong").output().unwrap();
            assert!(
                out.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            return;
        }
        let executable = std::env::var(MARKER).unwrap();
        let root = std::path::Path::new(&executable).parent().unwrap();
        let mut child = tokio::process::Command::new(&executable);
        child
            .env("GROK_CLI_CHAT_PROXY_BASE_URL", "synthetic-late")
            .env("XAI_API_KEY", "synthetic-late");
        super::super::enforce_subscription_child_env(&mut child, &executable, &[]).unwrap();
        // Exercise the spawn environment, not only the policy helper.
        use tokio::io::AsyncWriteExt;
        child
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped());
        let mut process = child.spawn().unwrap();
        process
            .stdin
            .take()
            .unwrap()
            .write_all(b"{\"id\":1}\n")
            .await
            .unwrap();
        let output = process.wait_with_output().await.unwrap();
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["result"]["home"], root.to_string_lossy().as_ref());
        assert!(result["result"]["key"].is_null());
        assert!(result["result"]["proxy"].is_null());
        assert_eq!(result["result"]["args"][0], "--leader-socket");
        std::fs::create_dir(root.join(".grok")).unwrap();
        std::fs::write(
            root.join(".grok/config.toml"),
            "[model.other]\napi_key='synthetic'\n",
        )
        .unwrap();
        assert!(super::super::validate_subscription_project_context("grok", root).is_err());
    }
}
