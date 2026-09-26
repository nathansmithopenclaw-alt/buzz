//! Composer dictation: local speech-to-text (Parakeet TDT 0.6B v2) biased
//! toward the channel's member names, plus an optional polish pass.
//!
//! Lifecycle, one session at a time across every window:
//! `dictation_start` (roster names) → `dictation_push_audio`* →
//! `dictation_stop` (transcript with roster names spliced in) →
//! `dictation_cleanup` (polished text or a typed failure the UI can explain).
//! `dictation_cancel` discards a session. Mentions are resolved locally in
//! the frontend at insert time; polish never adds or removes one.

mod capture;
mod catalog;
mod claude_plan;
mod cleanup;
pub(crate) mod cleanup_catalog;
mod disk_space;
mod engine;
mod model_store;
mod names;
mod secret_check;
mod settings;

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::app_state::AppState;
use crate::huddle::HuddlePhase;
use cleanup::{CleanupFailure, CleanupInput};
pub(crate) use settings::{CleanupSource, DictationSettings};

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

/// Lists curated speech models with readiness and available disk space.
#[tauri::command]
pub fn dictation_speech_catalog() -> Result<Vec<model_store::CatalogEntry>, String> {
    model_store::list()
}

/// The selected model's status, retained for composer compatibility.
#[tauri::command]
pub fn dictation_model_status(app: AppHandle) -> Result<serde_json::Value, String> {
    let settings = settings::load(&app)?;
    model_store::list()?
        .into_iter()
        .find(|entry| entry.id() == settings.speech.model_id())
        .map(|entry| serde_json::to_value(entry).map_err(|e| e.to_string()))
        .ok_or_else(|| "Unknown speech model".to_string())?
}

/// Starts a selected catalog download, using the saved model when omitted.
#[tauri::command]
pub fn dictation_download_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: Option<String>,
) -> Result<(), String> {
    let settings = settings::load(&app)?;
    model_store::start_download(
        model_id.as_deref().unwrap_or(settings.speech.model_id()),
        state.http_client.clone(),
    )
}

/// Removes one downloaded speech model while no dictation is using it.
#[tauri::command]
pub fn dictation_remove_model(model_id: String) -> Result<(), String> {
    let guard = active_session().lock().map_err(|e| e.to_string())?;
    if guard.is_some() {
        return Err("Stop dictation before removing a speech model".into());
    }
    model_store::remove(&model_id)
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

/// Roster names to bias recognition toward (vocabulary is reserved).
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
    let settings = settings::load(&app)?;
    let DictationContext { roster, vocabulary } = context.unwrap_or_default();
    let capture = capture::dir(&app).is_some();
    let (id, model_dir) = {
        let mut guard = active_session().lock().map_err(|e| e.to_string())?;
        if guard.is_some() {
            return Err("Dictation is already recording.".into());
        }
        let model_dir = model_store::resolve(settings.speech.model_id())?;
        let id = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
        *guard = Some(id);
        (id, model_dir)
    };
    let started = tauri::async_runtime::spawn_blocking(move || {
        engine::start(&model_dir, id, roster, capture)
    })
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
        let input = CleanupInput {
            transcript: String::new(),
            vocabulary,
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

/// Stops recording and returns the transcript. With captures on, the audio
/// and both decoding passes are written locally first.
#[tauri::command]
pub async fn dictation_stop(app: AppHandle, session_id: u64) -> Result<engine::Transcript, String> {
    if current_session() != Some(session_id) {
        return Err("this dictation is no longer recording".into());
    }
    let capture_dir = capture::dir(&app);
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut transcript = engine::stop(session_id)?;
        if let (Some(dir), Some(data)) = (capture_dir, transcript.capture.take()) {
            match capture::write(&dir, session_id, &transcript, &data) {
                Ok(id) => transcript.capture_id = Some(id),
                Err(error) => eprintln!("dictation: {error}"),
            }
        }
        Ok(transcript)
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|result| result);
    release_session(session_id);
    result
}

/// Appends a note (what was inserted, polished or sent) to a local capture.
#[tauri::command]
pub fn dictation_capture_note(
    app: AppHandle,
    capture_id: String,
    note: serde_json::Value,
) -> Result<(), String> {
    let dir = capture::dir(&app).ok_or("dictation captures are off")?;
    capture::note(&dir, &capture_id, note)
}

/// Discards a session (Esc, community switch, huddle start, unmount).
#[tauri::command]
pub fn dictation_cancel(session_id: u64) {
    engine::cancel(session_id);
    claude_plan::discard(session_id);
    release_session(session_id);
}

/// OpenRouter's OpenAI-compatible endpoint (the agent-settings source).
const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
/// Env var that holds the user's OpenRouter key in global agent settings.
const OPENROUTER_KEY_VAR: &str = "OPENROUTER_API_KEY";

/// The OpenRouter key from global agent env vars, if one is set.
fn agent_openrouter_key(env_vars: &std::collections::BTreeMap<String, String>) -> Option<&str> {
    env_vars
        .get(OPENROUTER_KEY_VAR)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

/// Which keyless cleanup sources this install can use. Reports presence
/// only; never a key.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationCleanupSources {
    agent_open_router: bool,
}

/// Reports whether the global agent settings hold an OpenRouter key.
#[tauri::command]
pub fn dictation_cleanup_sources(app: AppHandle) -> Result<DictationCleanupSources, String> {
    let config = crate::managed_agents::global_config::load_global_agent_config(&app)?;
    Ok(DictationCleanupSources {
        agent_open_router: agent_openrouter_key(&config.env_vars).is_some(),
    })
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
    vocabulary: Vec<String>,
}

fn connection_cleanup_enabled(cloud_enabled: bool, base_url: &str) -> bool {
    cloud_enabled || crate::model_connections::is_local_model_host(base_url)
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
    let outcome = if !settings.cloud_cleanup_enabled
        && !matches!(settings.cleanup, CleanupSource::Connection { .. })
    {
        Err(CleanupFailure::NotConfigured)
    } else {
        match &settings.cleanup {
            CleanupSource::Off => Err(CleanupFailure::NotConfigured),
            CleanupSource::Connection {
                connection_id,
                model,
            } => match crate::model_connections::resolve(&app, connection_id) {
                Ok((connection, key))
                    if connection_cleanup_enabled(
                        settings.cloud_cleanup_enabled,
                        &connection.base_url,
                    ) =>
                {
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
                Ok(_) => Err(CleanupFailure::NotConfigured),
                Err(message) => Err(CleanupFailure::Unavailable { message }),
            },
            CleanupSource::AgentOpenRouter { model } => {
                match crate::managed_agents::global_config::load_global_agent_config(&app) {
                    Ok(config) => match agent_openrouter_key(&config.env_vars) {
                        Some(key) => {
                            cleanup::run(
                                &state.http_client,
                                OPENROUTER_BASE_URL,
                                Some(key),
                                model,
                                &input,
                                timeout,
                            )
                            .await
                        }
                        None => Err(CleanupFailure::Unavailable {
                            message: "no OPENROUTER_API_KEY in Agent settings".into(),
                        }),
                    },
                    Err(message) => Err(CleanupFailure::Unavailable { message }),
                }
            }
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
    fn cloud_switch_does_not_disable_a_local_connection() {
        assert!(connection_cleanup_enabled(
            false,
            "http://100.80.202.75:8010/v1"
        ));
        assert!(!connection_cleanup_enabled(
            false,
            "https://openrouter.ai/api/v1"
        ));
        assert!(connection_cleanup_enabled(
            true,
            "https://openrouter.ai/api/v1"
        ));
    }

    #[test]
    fn agent_openrouter_key_requires_a_non_blank_value() {
        let mut vars = std::collections::BTreeMap::new();
        assert_eq!(agent_openrouter_key(&vars), None);
        vars.insert(OPENROUTER_KEY_VAR.to_string(), "   ".to_string());
        assert_eq!(agent_openrouter_key(&vars), None);
        vars.insert(OPENROUTER_KEY_VAR.to_string(), " sk-or-v1-x ".to_string());
        assert_eq!(agent_openrouter_key(&vars), Some("sk-or-v1-x"));
    }

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
