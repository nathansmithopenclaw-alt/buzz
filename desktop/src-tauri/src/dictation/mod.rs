//! Composer dictation: local speech-to-text (Parakeet TDT 0.6B v2) plus an
//! optional cleanup pass that marks addressed roster members as `@[Name]`.
//!
//! Lifecycle, one session at a time across every window:
//! `dictation_start` → `dictation_push_audio`* → `dictation_stop` (raw text)
//! → `dictation_cleanup` (cleaned text or a typed failure the UI can explain).
//! `dictation_cancel` discards a session. Mention validation, insertion and
//! the send-time checks live in the frontend mention pipeline.

mod claude_plan;
mod cleanup;
mod engine;
mod secret_check;
mod settings;

use std::{
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::app_state::AppState;
use crate::huddle::{
    models::{global_model_manager, ModelStatus},
    HuddlePhase,
};
use cleanup::{CleanupFailure, CleanupInput};
pub(crate) use settings::{CleanupSource, DictationSettings};

/// Bytes the dictation model needs: archive (482 MB) + extracted files
/// (661 MB) + a 1 GiB margin so the download never fills the disk.
const MODEL_REQUIRED_BYTES: u64 = 482_468_385 + 661_438_464 + 1024 * 1024 * 1024;

static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

fn active_session() -> &'static Mutex<Option<u64>> {
    static ACTIVE: OnceLock<Mutex<Option<u64>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(None))
}

fn current_session() -> Option<u64> {
    active_session().lock().ok().and_then(|guard| *guard)
}

fn release_session(id: u64) {
    if let Ok(mut guard) = active_session().lock() {
        if *guard == Some(id) {
            *guard = None;
        }
    }
}

/// Free bytes on the volume holding `path` (via `df`; `None` when unknown).
fn free_bytes(path: &Path) -> Option<u64> {
    if cfg!(windows) {
        return None;
    }
    let probe = if path.exists() {
        path.to_path_buf()
    } else {
        path.ancestors().find(|p| p.exists())?.to_path_buf()
    };
    let output = std::process::Command::new("df")
        .arg("-Pk")
        .arg(&probe)
        .output()
        .ok()?;
    parse_df_available_kib(&String::from_utf8_lossy(&output.stdout)).map(|kib| kib * 1024)
}

/// Parses the "Available" column (KiB) from POSIX `df -Pk` output.
fn parse_df_available_kib(output: &str) -> Option<u64> {
    output
        .lines()
        .nth(1)?
        .split_whitespace()
        .nth(3)?
        .parse()
        .ok()
}

/// Dictation model readiness plus the disk-space check shown before download.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationModelInfo {
    status: ModelStatus,
    required_bytes: u64,
    free_bytes: Option<u64>,
    enough_space: bool,
}

/// Returns the dictation model status and free disk space.
#[tauri::command]
pub fn dictation_model_status() -> Result<DictationModelInfo, String> {
    let manager = global_model_manager().ok_or("model storage is unavailable")?;
    let free = free_bytes(manager.models_dir_path());
    Ok(DictationModelInfo {
        status: manager.dictation_stt_status(),
        required_bytes: MODEL_REQUIRED_BYTES,
        free_bytes: free,
        enough_space: free.is_none_or(|bytes| bytes >= MODEL_REQUIRED_BYTES),
    })
}

/// Starts the one-time dictation model download after a free-space check.
#[tauri::command]
pub fn dictation_download_model(state: State<'_, AppState>) -> Result<(), String> {
    let manager = global_model_manager().ok_or("model storage is unavailable")?;
    if let Some(free) = free_bytes(manager.models_dir_path()) {
        if free < MODEL_REQUIRED_BYTES {
            return Err(format!(
                "Dictation needs about {:.1} GB free to download its speech model; this Mac has {:.1} GB.",
                MODEL_REQUIRED_BYTES as f64 / 1e9,
                free as f64 / 1e9
            ));
        }
    }
    manager.start_dictation_stt_download(state.http_client.clone());
    Ok(())
}

/// Reads dictation settings (defaults when none are saved).
#[tauri::command]
pub fn get_dictation_settings(app: AppHandle) -> Result<DictationSettings, String> {
    settings::load(&app)
}

/// Validates and saves dictation settings.
#[tauri::command]
pub fn save_dictation_settings(
    app: AppHandle,
    settings: DictationSettings,
) -> Result<DictationSettings, String> {
    settings::save(&app, &settings)?;
    Ok(settings)
}

/// Roster and vocabulary for the cleanup prompt, captured at start.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationContext {
    #[serde(default)]
    roster: Vec<String>,
    #[serde(default)]
    vocabulary: Vec<String>,
}

/// Returned by `dictation_start`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationStarted {
    session_id: u64,
}

fn huddle_is_live(state: &AppState) -> Result<bool, String> {
    Ok(!matches!(state.huddle()?.phase, HuddlePhase::Idle))
}

/// Starts a dictation session. Refused during a huddle (the mic would be live
/// to the room) or while another dictation is recording in any window.
#[tauri::command]
pub async fn dictation_start(
    app: AppHandle,
    state: State<'_, AppState>,
    context: Option<DictationContext>,
) -> Result<DictationStarted, String> {
    if huddle_is_live(&state)? {
        return Err("Dictation is off during a huddle.".into());
    }
    let manager = global_model_manager().ok_or("model storage is unavailable")?;
    let model_dir = manager
        .dictation_stt_model_dir()
        .ok_or("The dictation speech model is not downloaded yet.")?;
    let id = {
        let mut guard = active_session().lock().map_err(|e| e.to_string())?;
        if guard.is_some() {
            return Err("Dictation is already recording.".into());
        }
        let id = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
        *guard = Some(id);
        id
    };
    let started = tauri::async_runtime::spawn_blocking(move || engine::start(&model_dir, id))
        .await
        .map_err(|e| e.to_string())
        .and_then(|result| result);
    if let Err(error) = started {
        release_session(id);
        return Err(error);
    }

    // Start the Claude CLI now so its startup overlaps the user's speech.
    let settings = settings::load(&app)?;
    if let (true, CleanupSource::ClaudePlan { account_id, model }) =
        (settings.cloud_cleanup_enabled, &settings.cleanup)
    {
        let context = context.unwrap_or_default();
        let input = CleanupInput {
            transcript: String::new(),
            roster: context.roster,
            vocabulary: context.vocabulary,
            mode_instructions: None,
        };
        claude_plan::prewarm(app.clone(), id, account_id.clone(), model.clone(), input);
    }
    Ok(DictationStarted { session_id: id })
}

/// Receives a raw batch of 48 kHz f32 little-endian mono samples.
#[tauri::command]
pub fn dictation_push_audio(request: tauri::ipc::Request<'_>) -> Result<(), String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected raw binary body".into());
    };
    if bytes.len() > engine::MAX_AUDIO_BATCH_BYTES || bytes.len() % 4 != 0 {
        return Err("invalid dictation audio batch".into());
    }
    let id = current_session().ok_or("dictation is not recording")?;
    engine::push_audio(id, bytes.clone());
    Ok(())
}

/// Stops recording and returns the raw transcript.
#[tauri::command]
pub async fn dictation_stop(session_id: u64) -> Result<engine::Transcript, String> {
    if current_session() != Some(session_id) {
        return Err("this dictation is no longer recording".into());
    }
    let result = tauri::async_runtime::spawn_blocking(move || engine::stop(session_id))
        .await
        .map_err(|e| e.to_string())
        .and_then(|result| result);
    release_session(session_id);
    result
}

/// Discards a session (Esc, community switch, huddle start, unmount).
#[tauri::command]
pub fn dictation_cancel(session_id: u64) {
    engine::cancel(session_id);
    claude_plan::discard(session_id);
    release_session(session_id);
}

/// Cleanup outcome; `text` is `None` when the UI should use the raw words.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationCleanupResult {
    text: Option<String>,
    failure: Option<CleanupFailure>,
    elapsed_ms: u64,
}

/// Request for `dictation_cleanup`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationCleanupRequest {
    session_id: u64,
    transcript: String,
    #[serde(default)]
    roster: Vec<String>,
    #[serde(default)]
    vocabulary: Vec<String>,
}

/// Cleans up a transcript with the configured source. Never errors for
/// expected failures: they come back as a typed `failure` for the fallback note.
#[tauri::command]
pub async fn dictation_cleanup(
    app: AppHandle,
    state: State<'_, AppState>,
    request: DictationCleanupRequest,
) -> Result<DictationCleanupResult, String> {
    let settings = settings::load(&app)?;
    let started = Instant::now();
    let input = CleanupInput {
        transcript: request.transcript,
        roster: request.roster,
        vocabulary: request.vocabulary,
        mode_instructions: None,
    };
    let timeout = Duration::from_millis(u64::from(settings.timeout_ms));
    if !matches!(settings.cleanup, CleanupSource::ClaudePlan { .. })
        || !settings.cloud_cleanup_enabled
    {
        // The source changed since start (or cloud is off): end any launch.
        claude_plan::discard(request.session_id);
    }
    let outcome = if !settings.cloud_cleanup_enabled {
        Err(CleanupFailure::NotConfigured)
    } else {
        match &settings.cleanup {
            CleanupSource::Off => Err(CleanupFailure::NotConfigured),
            CleanupSource::Connection {
                connection_id,
                model,
            } => match crate::model_connections::resolve(&app, connection_id) {
                Ok((connection, key)) => {
                    cleanup::run(
                        &state.http_client,
                        &connection.base_url,
                        key.as_deref(),
                        model,
                        &input,
                        timeout,
                    )
                    .await
                }
                Err(message) => Err(CleanupFailure::Unavailable { message }),
            },
            CleanupSource::ClaudePlan { account_id, model } => {
                claude_plan::run(
                    &app,
                    request.session_id,
                    account_id.as_deref(),
                    model,
                    &input,
                    timeout,
                )
                .await
            }
        }
    };
    let elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(match outcome {
        Ok(text) => DictationCleanupResult {
            text: Some(text),
            failure: None,
            elapsed_ms,
        },
        Err(failure) => DictationCleanupResult {
            text: None,
            failure: Some(failure),
            elapsed_ms,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_df_available_column() {
        let output = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
                      /dev/disk3s5 1942700360 1300000000 409600000 77% /System/Volumes/Data\n";
        assert_eq!(parse_df_available_kib(output), Some(409_600_000));
        assert_eq!(parse_df_available_kib(""), None);
        assert_eq!(parse_df_available_kib("header only\n"), None);
    }

    // Compile-time: the requirement stays archive + extracted + ~1 GB margin.
    const _: () = assert!(MODEL_REQUIRED_BYTES > 2_000_000_000);
    const _: () = assert!(MODEL_REQUIRED_BYTES < 2_500_000_000);

    #[test]
    fn one_session_at_a_time_and_release_is_scoped() {
        {
            let mut guard = active_session().lock().unwrap();
            *guard = Some(41);
        }
        release_session(40);
        assert_eq!(
            current_session(),
            Some(41),
            "a stale id cannot release another session"
        );
        release_session(41);
        assert_eq!(current_session(), None);
    }
}
