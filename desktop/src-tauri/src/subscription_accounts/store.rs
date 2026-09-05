//! Versioned, private non-secret metadata. Token credentials live in separate files.
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use super::SubscriptionAccount;

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Registry {
    pub schema_version: u32,
    #[serde(default)]
    pub enabled: bool,
    pub accounts: Vec<SubscriptionAccount>,
}

pub(super) fn root(app: &AppHandle) -> Result<PathBuf, String> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|_| "App data directory unavailable")?;
    // Resolve the OS-owned data directory once. Reject a redirected immediate
    // application directory instead of silently hosting credentials elsewhere.
    let metadata = fs::symlink_metadata(&data).map_err(|_| "App data directory unavailable")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("App data directory must not be a symbolic link".into());
    }
    let data = fs::canonicalize(&data).map_err(|_| "App data directory unavailable")?;
    let root = data.join("subscription-accounts");
    private_dir(&root, true)?;
    Ok(root)
}

pub(super) fn private_dir(path: &Path, create: bool) -> Result<(), String> {
    if create && !path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .mode(0o700)
                .create(path)
                .map_err(|_| "Could not create private account directory")?;
        }
        #[cfg(not(unix))]
        return Err("Private account directories are not supported on this platform".into());
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| "Account directory unavailable")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Account directory must be an owned directory, not a link".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("Account directory permissions must be private (0700)".into());
        }
    }
    Ok(())
}

pub(super) fn home_at(root: &Path, id: &str) -> Result<PathBuf, String> {
    let parsed = uuid::Uuid::parse_str(id).map_err(|_| "Invalid account id")?;
    if parsed.to_string() != id {
        return Err("Invalid account id".into());
    }
    private_dir(root, false)?;
    let home = root.join(id);
    private_dir(&home, false)?;
    Ok(home)
}

pub(super) fn load(root: &Path) -> Result<Registry, String> {
    private_dir(root, false)?;
    let path = root.join("accounts.json");
    let meta = match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Registry {
                schema_version: 1,
                ..Registry::default()
            })
        }
        Err(_) => return Err("Account registry unavailable".into()),
        Ok(metadata) => metadata,
    };
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 1024 * 1024 {
        return Err("Invalid account registry".into());
    }
    let data = fs::read(path).map_err(|_| "Could not read account registry")?;
    let registry: Registry =
        serde_json::from_slice(&data).map_err(|_| "Invalid account registry")?;
    validate(&registry)?;
    Ok(registry)
}

pub(super) fn save(root: &Path, registry: &Registry) -> Result<(), String> {
    private_dir(root, false)?;
    validate(registry)?;
    let path = root.join("accounts.json");
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("Invalid account registry".into());
        }
        load(root)?;
    }
    let bytes =
        serde_json::to_vec_pretty(registry).map_err(|_| "Could not encode account registry")?;
    crate::managed_agents::atomic_write_json_restricted(&path, &bytes)
        .map_err(|_| "Could not persist account registry".into())
}

fn validate(registry: &Registry) -> Result<(), String> {
    if !matches!(registry.schema_version, 1 | 2) {
        return Err("Unsupported account registry version".into());
    }
    if registry.accounts.len() > 100 {
        return Err("Subscription account limit reached (100)".into());
    }
    let mut ids = HashSet::new();
    for account in &registry.accounts {
        if uuid::Uuid::parse_str(&account.id)
            .ok()
            .is_none_or(|id| id.to_string() != account.id)
            || !ids.insert(&account.id)
            || !matches!(
                (account.harness.as_str(), account.provider.as_str()),
                ("claude", "anthropic") | ("codex", "openai") | ("grok", "xai")
            )
            || !matches!(account.kind.as_str(), "login" | "token")
            || account.kind == "token"
                && (account.harness != "claude" || registry.schema_version < 2)
            || match (account.minted_at, account.expires_at) {
                (None, None) => false,
                (Some(minted), Some(expires)) => {
                    account.kind != "token"
                        || minted < 0
                        || minted.checked_add(super::tokens::LIFETIME_SECONDS) != Some(expires)
                }
                _ => true,
            }
            || !matches!(
                account.state.as_str(),
                "connecting" | "ready" | "needs_login" | "removal_pending" | "unavailable"
            )
            || account.nickname.as_ref().is_some_and(|name| {
                name.trim().is_empty()
                    || name.chars().count() > 80
                    || name.chars().any(char::is_control)
            })
            || account.logout_completed && account.state != "removal_pending"
        {
            return Err("Invalid subscription account metadata".into());
        }
    }
    Ok(())
}

pub(super) fn account_mut<'a>(
    registry: &'a mut Registry,
    id: &str,
) -> Result<&'a mut SubscriptionAccount, String> {
    registry
        .accounts
        .iter_mut()
        .find(|account| account.id == id)
        .ok_or_else(|| "Subscription account not found".into())
}
