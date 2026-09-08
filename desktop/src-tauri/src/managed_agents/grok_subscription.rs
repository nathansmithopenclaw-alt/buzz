//! Compose non-secret Grok settings without moving or inspecting credentials.
use buzz_core_pkg::grok_subscription::{competing_env, validate_config};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn read_config(path: &Path) -> Result<Option<String>, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Could not inspect Grok settings".into()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 1024 * 1024 {
        return Err("Grok settings must be a regular file under 1 MiB".into());
    }
    let raw = std::fs::read_to_string(path).map_err(|_| "Could not read Grok settings")?;
    let parsed: toml::Value = toml::from_str(&raw).map_err(|_| "Invalid Grok TOML settings")?;
    validate_config(&serde_json::to_value(parsed).map_err(|_| "Invalid Grok settings")?)?;
    Ok(Some(raw))
}

pub(super) fn configure(command: &mut Command, home: &Path) -> Result<(), String> {
    let source = super::subscription_binding::command_env(command, "GROK_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|p| p.join(".grok")))
        .ok_or("Cannot resolve Grok settings home")?;
    if !source.is_absolute() {
        return Err("Grok settings home must be absolute".into());
    }
    // Validate every layer before writing a composed profile.
    let mut configs = Vec::new();
    for name in ["config.toml", "managed_config.toml", "requirements.toml"] {
        configs.push((name, read_config(&source.join(name))?));
    }
    if let Some(cwd) = command.get_current_dir() {
        for parent in cwd.ancestors() {
            read_config(&parent.join(".grok/config.toml"))?;
        }
    }
    for name in ["config.toml", "managed_config.toml", "requirements.toml"] {
        read_config(&Path::new("/etc/grok").join(name))?;
    }
    for key in ["GROK_CONFIG", "GROK_CONFIG_PATH"] {
        if super::subscription_binding::command_env(command, key).is_some() {
            return Err("Remove the Grok config environment overlay before selecting a subscription account".into());
        }
    }
    for (name, raw) in configs {
        let mut value: toml::Value = raw
            .as_deref()
            .unwrap_or("")
            .parse()
            .map_err(|_| "Invalid Grok settings")?;
        if name == "requirements.toml" || name == "config.toml" {
            let table = value
                .as_table_mut()
                .ok_or("Grok settings must be a table")?;
            let auth = table
                .entry("auth")
                .or_insert_with(|| toml::Value::Table(Default::default()))
                .as_table_mut()
                .ok_or("Invalid Grok auth settings")?;
            auth.insert("preferred_method".into(), "oidc".into());
            let policy = table
                .entry("grok_com_config")
                .or_insert_with(|| toml::Value::Table(Default::default()))
                .as_table_mut()
                .ok_or("Invalid Grok auth policy")?;
            policy.insert("disable_api_key_auth".into(), true.into());
        }
        let bytes = toml::to_string(&value).map_err(|_| "Could not compose Grok settings")?;
        let path = home.join(name);
        if let Ok(m) = std::fs::symlink_metadata(&path) {
            if !m.is_file() || m.file_type().is_symlink() {
                return Err("Owned Grok settings must not be redirected".into());
            }
        }
        let mut file = atomic_write_file::AtomicWriteFile::open(&path)
            .map_err(|_| "Could not prepare Grok settings")?;
        use std::io::Write;
        file.write_all(bytes.as_bytes())
            .map_err(|_| "Could not write Grok settings")?;
        file.commit()
            .map_err(|_| "Could not commit Grok settings")?;
    }
    // These contain user extensions, never auth.json or native session history.
    #[cfg(unix)]
    for name in ["skills", "agents", "plugins"] {
        let original = source.join(name);
        let target = home.join(name);
        if let Ok(metadata) = std::fs::symlink_metadata(&target) {
            if !metadata.file_type().is_symlink()
                || std::fs::read_link(&target).ok().as_deref() != Some(original.as_path())
            {
                return Err("Grok extension source changed; restore its settings source before selecting this account".into());
            }
        } else if original.is_dir() {
            std::os::unix::fs::symlink(&original, &target)
                .map_err(|_| "Could not preserve Grok extensions")?;
        }
    }
    let names: Vec<_> = std::env::vars_os()
        .map(|(k, _)| k)
        .chain(command.get_envs().map(|(k, _)| k.to_owned()))
        .collect();
    for name in names {
        if competing_env(&name.to_string_lossy()) {
            command.env_remove(name);
        }
    }
    command.env("GROK_HOME", home);
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn composes_settings_without_credentials_or_session_history() {
        let source = tempfile::tempdir().unwrap();
        let account = tempfile::tempdir().unwrap();
        std::fs::write(
            source.path().join("config.toml"),
            "[models]\ndefault='grok-4.6'\n",
        )
        .unwrap();
        // Synthetic secret sentinel, never a provider credential.
        std::fs::write(source.path().join("auth.json"), "DO_NOT_COPY").unwrap();
        std::fs::create_dir(source.path().join("sessions")).unwrap();
        let mut command = Command::new("unused");
        command
            .env("GROK_HOME", source.path())
            .env("XAI_API_KEY", "synthetic");
        command
            .env_remove("GROK_CONFIG")
            .env_remove("GROK_CONFIG_PATH");
        configure(&mut command, account.path()).unwrap();
        let config = std::fs::read_to_string(account.path().join("config.toml")).unwrap();
        assert!(config.contains("grok-4.6"));
        assert!(config.contains("disable_api_key_auth = true"));
        assert!(!account.path().join("auth.json").exists());
        assert!(!account.path().join("sessions").exists());
        assert!(command
            .get_envs()
            .any(|(k, v)| k == "XAI_API_KEY" && v.is_none()));
        std::fs::write(
            source.path().join("config.toml"),
            "[model.other]\napi_key='synthetic'\n",
        )
        .unwrap();
        command.env("GROK_HOME", source.path());
        assert!(configure(&mut command, account.path()).is_err());
    }

    #[test]
    fn grok_has_native_account_runtime_identity_and_stdio_arguments() {
        let runtime = super::super::known_acp_runtime("/installed/bin/grok").unwrap();
        assert_eq!(runtime.id, "grok");
        assert_eq!(
            super::super::subscription_binding::account_harness(Some(runtime.id)),
            Some("grok")
        );
        assert_eq!(
            super::super::normalize_agent_args("grok", vec![]),
            ["agent", "--always-approve", "stdio"]
        );
    }
}
