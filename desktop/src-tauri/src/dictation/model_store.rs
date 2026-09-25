//! Catalog downloads: bounded, hash-checked, staged, and committed by rename.

use super::{
    catalog::{self, ModelFile, SpeechModel, DISK_MARGIN},
    disk_space,
};
use crate::huddle::models::{global_model_manager, ModelStatus};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Duration,
};
use tokio::io::AsyncWriteExt;

const MANIFEST: &str = ".buzz-model-manifest";
const LICENSE: &str = "MODEL_LICENSE.txt";

fn operations() -> &'static Mutex<HashMap<String, ModelStatus>> {
    static OPERATIONS: OnceLock<Mutex<HashMap<String, ModelStatus>>> = OnceLock::new();
    OPERATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn root() -> Result<PathBuf, String> {
    Ok(global_model_manager()
        .ok_or("Model storage is unavailable")?
        .models_dir_path()
        .to_path_buf())
}

pub(crate) fn ready_path(root: &Path, model: &SpeechModel) -> Option<PathBuf> {
    let path = root.join(model.directory);
    let version = std::fs::read_to_string(path.join(MANIFEST)).ok()?;
    let known_version =
        version == model.revision || (model.id == catalog::DEFAULT_MODEL_ID && version == "1");
    (known_version
        && path.join(LICENSE).is_file()
        && model.files.iter().all(|file| {
            std::fs::metadata(path.join(file.name))
                .is_ok_and(|meta| meta.is_file() && meta.len() == file.bytes)
        }))
    .then_some(path)
}

pub(crate) fn resolve(id: &str) -> Result<PathBuf, String> {
    ready_path(&root()?, catalog::model(id)?)
        .ok_or_else(|| "Download the selected speech model in Settings → Voice".into())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogEntry {
    #[serde(flatten)]
    model: &'static SpeechModel,
    size_bytes: u64,
    status: ModelStatus,
    required_bytes: u64,
    free_bytes: Option<u64>,
    enough_space: bool,
}

impl CatalogEntry {
    pub(crate) fn id(&self) -> &str {
        self.model.id
    }
}

pub(crate) fn list() -> Result<Vec<CatalogEntry>, String> {
    let root = root()?;
    let free = disk_space::available(&root);
    let operations = operations().lock().map_err(|e| e.to_string())?;
    Ok(catalog::MODELS
        .iter()
        .map(|model| CatalogEntry {
            model,
            size_bytes: model.size_bytes(),
            status: if let Some(ModelStatus::Downloading { progress_percent }) =
                operations.get(model.id)
            {
                ModelStatus::Downloading {
                    progress_percent: *progress_percent,
                }
            } else if ready_path(&root, model).is_some() {
                ModelStatus::Ready
            } else {
                match operations.get(model.id) {
                    Some(ModelStatus::Error(error)) => ModelStatus::Error(error.clone()),
                    _ => ModelStatus::NotDownloaded,
                }
            },
            required_bytes: model.size_bytes() + DISK_MARGIN,
            free_bytes: free,
            enough_space: check_space(free, model.size_bytes()).is_ok(),
        })
        .collect())
}

fn check_space(free: Option<u64>, bytes: u64) -> Result<(), String> {
    let free = free.ok_or("Cannot check available disk space. Try again before downloading.")?;
    let required = bytes.saturating_add(DISK_MARGIN);
    if free < required {
        return Err(format!(
            "Not enough space · {:.2} GB required · {:.2} GB available",
            required as f64 / 1e9,
            free as f64 / 1e9
        ));
    }
    Ok(())
}

pub(crate) fn start_download(id: &str, client: reqwest::Client) -> Result<(), String> {
    let model = catalog::model(id)?;
    let root = root()?;
    let mut state = operations().lock().map_err(|e| e.to_string())?;
    if ready_path(&root, model).is_some() {
        return Ok(());
    }
    if state
        .values()
        .any(|status| matches!(status, ModelStatus::Downloading { .. }))
    {
        return Err("A speech model is already downloading".into());
    }
    check_space(disk_space::available(&root), model.size_bytes())?;
    state.insert(
        id.to_string(),
        ModelStatus::Downloading {
            progress_percent: 0,
        },
    );
    tauri::async_runtime::spawn(async move {
        let result = install(
            &root,
            model,
            &client,
            disk_space::available(&root),
            |file| model.file_url(file),
            |percent| {
                if let Ok(mut state) = operations().lock() {
                    state.insert(
                        model.id.to_string(),
                        ModelStatus::Downloading {
                            progress_percent: percent,
                        },
                    );
                }
            },
        )
        .await;
        if let Ok(mut state) = operations().lock() {
            state.insert(
                model.id.to_string(),
                match result {
                    Ok(()) => ModelStatus::Ready,
                    Err(error) => ModelStatus::Error(error),
                },
            );
        }
    });
    Ok(())
}

pub(crate) fn remove(id: &str) -> Result<(), String> {
    let model = catalog::model(id)?;
    let mut state = operations().lock().map_err(|e| e.to_string())?;
    if matches!(state.get(id), Some(ModelStatus::Downloading { .. })) {
        return Err("Wait for the download to finish before removing this model".into());
    }
    remove_from(&root()?, model)?;
    state.remove(id);
    Ok(())
}

fn remove_from(root: &Path, model: &SpeechModel) -> Result<(), String> {
    let path = root.join(model.directory);
    if path.exists() {
        std::fs::remove_dir_all(path).map_err(|e| format!("Could not remove speech model: {e}"))?;
    }
    Ok(())
}

async fn install(
    root: &Path,
    model: &SpeechModel,
    client: &reqwest::Client,
    available_bytes: Option<u64>,
    url: impl Fn(&ModelFile) -> String,
    progress: impl Fn(u8),
) -> Result<(), String> {
    check_space(available_bytes, model.size_bytes())?;
    tokio::fs::create_dir_all(root)
        .await
        .map_err(|e| e.to_string())?;
    let stage = tempfile::Builder::new()
        .prefix(".dictation-download-")
        .tempdir_in(root)
        .map_err(|e| e.to_string())?;
    let mut downloaded = 0;
    for file in model.files {
        download_file(
            client,
            &url(file),
            &stage.path().join(file.name),
            file,
            |bytes| {
                progress(((downloaded + bytes) * 99 / model.size_bytes().max(1)) as u8);
            },
        )
        .await?;
        downloaded += file.bytes;
    }
    tokio::fs::write(stage.path().join(LICENSE), model.attribution())
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::write(stage.path().join(MANIFEST), model.revision)
        .await
        .map_err(|e| e.to_string())?;
    let final_path = root.join(model.directory);
    // Only incomplete caches can exist here; preserve them until the full
    // replacement is verified, and restore them if the commit fails.
    let backup = tempfile::Builder::new()
        .prefix(".dictation-backup-")
        .tempdir_in(root)
        .map_err(|e| e.to_string())?;
    let previous = backup.path().join("previous");
    let had_previous = final_path.exists();
    if had_previous {
        tokio::fs::rename(&final_path, &previous)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Err(error) = tokio::fs::rename(stage.path(), &final_path).await {
        if had_previous {
            if let Err(restore) = tokio::fs::rename(&previous, &final_path).await {
                let retained = backup.keep();
                return Err(format!("Install failed: {error}; recovery failed: {restore}. Previous files retained at {}", retained.display()));
            }
        }
        return Err(format!("Could not install speech model: {error}"));
    }
    Ok(())
}

async fn download_file(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    file: &ModelFile,
    progress: impl Fn(u64),
) -> Result<(), String> {
    let mut response = client
        .get(url)
        .timeout(Duration::from_secs(30 * 60))
        .send()
        .await
        .map_err(|e| format!("Speech model download failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Speech model download failed (HTTP {})",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|bytes| bytes != file.bytes)
    {
        return Err("Speech model download has an unexpected size".into());
    }
    let mut output = tokio::fs::File::create(path)
        .await
        .map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut received = 0;
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        received += chunk.len() as u64;
        if received > file.bytes {
            return Err("Speech model download exceeded its expected size".into());
        }
        digest.update(&chunk);
        output.write_all(&chunk).await.map_err(|e| e.to_string())?;
        progress(received);
    }
    output.sync_all().await.map_err(|e| e.to_string())?;
    if received != file.bytes || hex::encode(digest.finalize()) != file.sha256 {
        return Err("Speech model verification failed. Download it again.".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "model_store_tests.rs"]
mod tests;
