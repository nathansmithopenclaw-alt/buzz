//! Device-local model connections. Secrets never appear in catalog IPC responses.
mod local_host;
pub(crate) use local_host::is_local_model_host;
mod local_permission;
pub(crate) mod network;
mod private_network;
mod probe;
pub(crate) mod qualification;
pub(crate) mod runtime;
pub(crate) mod source;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Mutex};
use tauri::{AppHandle, Manager};
static STORE_LOCK: Mutex<()> = Mutex::new(());
pub const CONNECTION_ENV: &str = "BUZZ_MODEL_CONNECTION_ID";
#[derive(Clone, Serialize, Deserialize)]
pub struct ModelConnection {
    pub id: String,
    pub name: String,
    pub base_url: String,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub has_key: bool,
    #[serde(default)]
    pub tested_model: Option<String>,
    #[serde(default)]
    pub tested_at: Option<u64>,
    #[serde(default)]
    pub qualification: Option<qualification::Qualification>,
    #[serde(default, skip_serializing)]
    secret_ref: Option<String>,
}
// Persist the secret reference, but never expose it in public responses.
#[derive(Serialize, Deserialize)]
struct StoredConnection {
    connection: ModelConnection,
    secret_ref: Option<String>,
}
fn path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("model-connections.json"))
}
fn load(app: &AppHandle) -> Result<Vec<ModelConnection>, String> {
    load_file(&path(app)?)
}
fn load_file(path: &std::path::Path) -> Result<Vec<ModelConnection>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Model connection store is too large".into());
    }
    let stored: Vec<StoredConnection> =
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(stored
        .into_iter()
        .map(|mut row| {
            row.connection.secret_ref = row.secret_ref;
            row.connection
        })
        .collect())
}
fn persist(app: &AppHandle, rows: &[ModelConnection]) -> Result<(), String> {
    persist_file(&path(app)?, rows)
}
fn persist_file(path: &std::path::Path, rows: &[ModelConnection]) -> Result<(), String> {
    let stored: Vec<_> = rows
        .iter()
        .map(|row| StoredConnection {
            connection: row.clone(),
            secret_ref: row.secret_ref.clone(),
        })
        .collect();
    let bytes = serde_json::to_vec_pretty(&stored).map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Model connection store exceeds 1 MiB".into());
    }
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    crate::model_connections::runtime::write_private(&temp, &bytes)?;
    std::fs::rename(&temp, path).map_err(|e| e.to_string())
}
fn secret(row: &ModelConnection) -> Result<Option<String>, String> {
    row.secret_ref
        .as_deref()
        .map(|key| crate::secret_store::SecretStore::shared("buzz-desktop").load(key))
        .transpose()
        .map(Option::flatten)
}
pub fn resolve(app: &AppHandle, id: &str) -> Result<(ModelConnection, Option<String>), String> {
    let row = find_connection(app, id)?;
    let key = secret(&row)?;
    if row.has_key && key.is_none() {
        return Err("Model connection key is unavailable. Reconnect in Model connections.".into());
    }
    Ok((row, key))
}
fn find_connection(app: &AppHandle, id: &str) -> Result<ModelConnection, String> {
    load(app)?
        .into_iter()
        .find(|row| row.id == id)
        .ok_or_else(|| "Model connection was removed. Choose another connection.".into())
}
fn key_for_save(
    replacement: Option<String>,
    saved: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<Option<String>, String> {
    // An explicit replacement or clearing must recover an unavailable saved key.
    Ok(match replacement {
        Some(key) => Some(key),
        None => saved()?,
    }
    .filter(|key| !key.is_empty()))
}
#[tauri::command]
pub fn list_model_connections(app: AppHandle) -> Result<Vec<ModelConnection>, String> {
    load(&app)
}
#[derive(Deserialize)]
pub struct SaveConnectionInput {
    pub id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub api_key: Option<String>,
}
#[tauri::command]
pub async fn save_model_connection(
    app: AppHandle,
    input: SaveConnectionInput,
) -> Result<ModelConnection, String> {
    if input
        .api_key
        .as_ref()
        .is_some_and(|key| key.len() > 8192 || key.contains(['\r', '\n']))
    {
        return Err("API key is invalid or too long".into());
    }
    let name = input.name.trim().to_string();
    if name.is_empty() || name.len() > 100 {
        return Err("Enter a connection name (up to 100 characters)".into());
    }
    let base_url = probe::normalize_url(&input.base_url)?;
    let old = input
        .id
        .as_deref()
        .map(|id| find_connection(&app, id))
        .transpose()?;
    if old
        .as_ref()
        .is_some_and(|row| row.base_url != base_url && row.has_key)
        && input.api_key.is_none()
    {
        return Err("Re-enter the API key for the new server, or clear it explicitly. Saved keys are not sent to a different address.".into());
    }
    let key = key_for_save(input.api_key.clone(), || {
        old.as_ref()
            .map(|row| {
                let key = secret(row)?;
                if row.has_key && key.is_none() {
                    return Err(
                        "Saved key is unavailable. Enter a new API key or remove it.".into(),
                    );
                }
                Ok(key)
            })
            .transpose()
            .map(Option::flatten)
    })?;
    let models = probe::catalog(&base_url, key.as_deref(), 5).await?;
    let _guard = STORE_LOCK.lock().map_err(|e| e.to_string())?;
    let mut rows = load(&app)?;
    if let Some(previous) = &old {
        let current = rows
            .iter()
            .find(|row| row.id == previous.id)
            .ok_or("Connection was removed while connecting")?;
        if current.base_url != previous.base_url
            || current.name != previous.name
            || current.secret_ref != previous.secret_ref
        {
            return Err("Connection changed while connecting. Reopen it and try again.".into());
        }
    }
    if rows.len() >= 64 && input.id.is_none() {
        return Err("Maximum of 64 model connections reached".into());
    }
    let secret_ref = if let Some(key) = &key {
        let reference = format!("model-connection:{}", uuid::Uuid::new_v4());
        crate::secret_store::SecretStore::shared("buzz-desktop").store(&reference, key)?;
        Some(reference)
    } else {
        None
    };
    let row = ModelConnection {
        id: input.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        name,
        base_url,
        models,
        has_key: key.is_some(),
        secret_ref,
        tested_model: None,
        tested_at: None,
        qualification: None,
    };
    rows.retain(|existing| existing.id != row.id);
    rows.push(row.clone());
    if let Err(error) = persist(&app, &rows) {
        if let Some(reference) = &row.secret_ref {
            let _ = crate::secret_store::SecretStore::shared("buzz-desktop").delete(reference);
        }
        return Err(error);
    }
    if let Some(reference) = old.and_then(|row| row.secret_ref) {
        let _ = crate::secret_store::SecretStore::shared("buzz-desktop").delete(&reference);
    }
    Ok(row)
}
#[tauri::command]
pub fn remove_model_connection(app: AppHandle, id: String) -> Result<(), String> {
    let _guard = STORE_LOCK.lock().map_err(|e| e.to_string())?;
    let mut rows = load(&app)?;
    let old = rows.iter().find(|row| row.id == id).cloned();
    rows.retain(|row| row.id != id);
    persist(&app, &rows)?;
    if let Some(reference) = old.and_then(|row| row.secret_ref) {
        crate::secret_store::SecretStore::shared("buzz-desktop").delete(&reference)?;
    }
    Ok(())
}
#[tauri::command]
pub async fn check_model_connection(app: AppHandle, id: String) -> Result<Vec<String>, String> {
    static LIMIT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(8);
    let _permit = LIMIT
        .acquire()
        .await
        .map_err(|_| "Model checks are unavailable")?;
    let (row, key) = resolve(&app, &id)?;
    probe::catalog(&row.base_url, key.as_deref(), 5).await
}
#[tauri::command]
pub async fn test_model_connection(
    app: AppHandle,
    id: String,
    model: String,
) -> Result<ModelConnection, String> {
    // A shared local endpoint may also serve production agents. Explicit tests
    // are bounded and serialized across connection dialogs and windows.
    static LIMIT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
    let _permit = LIMIT
        .acquire()
        .await
        .map_err(|_| "Model tests are unavailable")?;
    let (mut row, key) = resolve(&app, &id)?;
    let (models, limits) = probe::catalog_with_context(&row.base_url, key.as_deref(), 5).await?;
    if !models.contains(&model) {
        return Err("This model is no longer available. Refresh the connection.".into());
    }
    let qualified = tokio::time::timeout(
        std::time::Duration::from_secs(180),
        qualification::qualify(
            &row.base_url,
            key.as_deref(),
            &model,
            limits.get(&model).copied(),
        ),
    )
    .await
    .map_err(|_| {
        "Model qualification exceeded three minutes. Try again when the server is less busy."
    })?;
    let _guard = STORE_LOCK.lock().map_err(|e| e.to_string())?;
    let mut rows = load(&app)?;
    let current = rows
        .iter_mut()
        .find(|r| r.id == id)
        .ok_or("Connection was removed during test")?;
    if current.base_url != row.base_url
        || current.secret_ref != row.secret_ref
        || current.name != row.name
    {
        return Err("Connection changed during test. Test again.".into());
    }
    row.models = models;
    row.qualification = Some(qualified);
    row.tested_model = row
        .qualification
        .as_ref()
        .filter(|q| {
            q.protocols.iter().any(|check| {
                q.check(&model, check.protocol)
                    .is_some_and(|check| check.tool_round_trip)
            })
        })
        .map(|_| model);
    row.tested_at = Some(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs(),
    );
    *current = row.clone();
    persist(&app, &rows)?;
    Ok(row)
}
pub async fn discover_for_env(
    app: &AppHandle,
    env: &BTreeMap<String, String>,
) -> Result<Option<Vec<String>>, String> {
    let Some(id) = env.get(CONNECTION_ENV).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    // Pickers use the last saved catalog. Refresh/Test are explicit operations;
    // opening an editor neither contacts the endpoint nor reads credentials.
    Ok(Some(find_connection(app, id)?.models))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacing_or_clearing_a_key_recovers_without_reading_the_old_secret() {
        let unavailable = || Err("saved key is unavailable".to_string());
        assert_eq!(
            key_for_save(Some("replacement".into()), unavailable).unwrap(),
            Some("replacement".into())
        );
        assert_eq!(
            key_for_save(Some(String::new()), unavailable).unwrap(),
            None
        );
        assert!(key_for_save(None, unavailable).is_err());
    }
    #[test]
    fn saved_connections_round_trip_without_exposing_keys_and_removal_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        let row = ModelConnection {
            id: "stable-id".into(),
            name: "Spark".into(),
            base_url: "http://127.0.0.1:8010/v1".into(),
            models: vec!["Qwen/model".into()],
            has_key: true,
            secret_ref: Some("key-reference".into()),
            tested_model: None,
            tested_at: None,
            qualification: None,
        };
        persist_file(&path, &[row]).unwrap();
        let restored = load_file(&path).unwrap();
        assert_eq!(restored[0].id, "stable-id");
        assert_eq!(restored[0].secret_ref.as_deref(), Some("key-reference"));
        assert!(!serde_json::to_string(&restored)
            .unwrap()
            .contains("key-reference"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        persist_file(&path, &[]).unwrap();
        assert!(load_file(&path).unwrap().is_empty());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn invalid_store_is_not_treated_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        std::fs::write(&path, "broken").unwrap();
        assert!(load_file(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken");
    }
}
