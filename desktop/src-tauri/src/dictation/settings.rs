//! Device-local dictation settings (`<app data>/dictation-settings.json`).
//!
//! Stage 1 stores the cleanup source, timeout, hotkey and language. The file is
//! versioned and schema-validated on load; an unreadable or invalid file falls
//! back to defaults instead of breaking dictation.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const SETTINGS_FILE: &str = "dictation-settings.json";
const SETTINGS_VERSION: u32 = 2;
const MAX_SETTINGS_BYTES: u64 = 64 * 1024;

/// Default cleanup deadline; past it the user gets the on-device result.
pub(crate) const DEFAULT_TIMEOUT_MS: u32 = 4_000;
const MIN_TIMEOUT_MS: u32 = 1_000;
const MAX_TIMEOUT_MS: u32 = 15_000;
/// `KeyboardEvent.code` of the default dictation key (Right Option / Right Alt).
pub(crate) const DEFAULT_HOTKEY: &str = "AltRight";

/// Where cleanup runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum CleanupSource {
    /// Verbatim: no model; Buzz's matcher proposes mention suggestions only.
    Off,
    /// An OpenAI-compatible model connection (e.g. OpenRouter).
    #[serde(rename_all = "camelCase")]
    Connection {
        connection_id: String,
        model: String,
    },
    /// OpenRouter with the `OPENROUTER_API_KEY` already in the user's global
    /// agent settings, so no key is entered twice. The key is read at request
    /// time and never stored or returned by dictation.
    #[serde(rename_all = "camelCase")]
    AgentOpenRouter { model: String },
    /// The user's own Claude plan through the unmodified Claude Code CLI.
    #[serde(rename_all = "camelCase")]
    ClaudePlan {
        /// `None` uses the default Claude account.
        account_id: Option<String>,
        model: String,
    },
}

/// Local speech model selection; the tag permits future engine families.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SpeechSource {
    #[serde(rename_all = "camelCase")]
    Local { model_id: String },
}

impl Default for SpeechSource {
    fn default() -> Self {
        Self::Local {
            model_id: super::catalog::DEFAULT_MODEL_ID.into(),
        }
    }
}

impl SpeechSource {
    pub(crate) fn model_id(&self) -> &str {
        match self {
            Self::Local { model_id } => model_id,
        }
    }
}

/// Persisted dictation settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DictationSettings {
    pub version: u32,
    #[serde(default)]
    pub speech: SpeechSource,
    pub cleanup: CleanupSource,
    pub timeout_ms: u32,
    /// `KeyboardEvent.code` for push-to-talk / tap-to-toggle.
    pub hotkey: String,
    /// BCP-47 language of speech ("en" for Parakeet 0.6B v2).
    pub language: String,
    /// Master switch for any cloud cleanup call.
    pub cloud_cleanup_enabled: bool,
}

impl Default for DictationSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            speech: SpeechSource::default(),
            cleanup: CleanupSource::Off,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            hotkey: DEFAULT_HOTKEY.to_string(),
            language: "en".to_string(),
            cloud_cleanup_enabled: true,
        }
    }
}

fn is_valid_key_code(code: &str) -> bool {
    // Typing, navigation, help, reload, menus and developer shortcuts stay free.
    matches!(code, "AltLeft" | "AltRight" | "F8" | "F9")
        || code
            .strip_prefix('F')
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| (13..=24).contains(&n) && code == format!("F{n}"))
}

fn bounded_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

impl DictationSettings {
    /// Rejects values that could not have come from the settings UI.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.version != SETTINGS_VERSION {
            return Err(format!(
                "unsupported dictation settings version {}",
                self.version
            ));
        }
        if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&self.timeout_ms) {
            return Err(format!(
                "cleanup timeout must be between {MIN_TIMEOUT_MS} and {MAX_TIMEOUT_MS} ms"
            ));
        }
        super::catalog::model(self.speech.model_id())?;
        if !is_valid_key_code(&self.hotkey) {
            return Err("This key conflicts with typing or another shortcut. Choose an Option/Alt key or F8, F9, F13–F24.".into());
        }
        if !bounded_text(&self.language, 16) {
            return Err("dictation language is invalid".into());
        }
        match &self.cleanup {
            CleanupSource::Off => {}
            CleanupSource::Connection {
                connection_id,
                model,
            } => {
                if !bounded_text(connection_id, 128) || !bounded_text(model, 200) {
                    return Err("choose a model connection and model for cleanup".into());
                }
            }
            CleanupSource::AgentOpenRouter { model } => {
                if !bounded_text(model, 200) {
                    return Err("choose an OpenRouter model for cleanup".into());
                }
            }
            CleanupSource::ClaudePlan { account_id, model } => {
                if account_id
                    .as_deref()
                    .is_some_and(|id| !bounded_text(id, 128))
                    || !bounded_text(model, 200)
                {
                    return Err("choose a Claude account and model for cleanup".into());
                }
            }
        }
        Ok(())
    }
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join(SETTINGS_FILE))
}

/// Loads settings, falling back to defaults for a missing or invalid file.
pub(crate) fn load_from_path(path: &Path) -> DictationSettings {
    let Ok(metadata) = std::fs::metadata(path) else {
        return DictationSettings::default();
    };
    if metadata.len() > MAX_SETTINGS_BYTES {
        eprintln!("buzz-desktop: dictation settings file too large; using defaults");
        return DictationSettings::default();
    }
    let parsed = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<DictationSettings>(&bytes).ok());
    match parsed {
        Some(mut settings) => {
            if settings.version == 1 {
                settings.version = SETTINGS_VERSION;
                if !is_valid_key_code(&settings.hotkey) {
                    settings.hotkey = DEFAULT_HOTKEY.into();
                }
            }
            if settings.validate().is_ok() {
                settings
            } else {
                DictationSettings::default()
            }
        }
        _ => {
            eprintln!("buzz-desktop: dictation settings invalid; using defaults");
            DictationSettings::default()
        }
    }
}

pub(crate) fn save_to_path(path: &Path, settings: &DictationSettings) -> Result<(), String> {
    settings.validate()?;
    let payload = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("could not encode dictation settings: {error}"))?;
    crate::managed_agents::atomic_write_json_restricted(path, &payload)
}

pub(crate) fn load(app: &AppHandle) -> Result<DictationSettings, String> {
    Ok(load_from_path(&settings_path(app)?))
}

pub(crate) fn save(app: &AppHandle, settings: &DictationSettings) -> Result<(), String> {
    save_to_path(&settings_path(app)?, settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_verbatim_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let settings = load_from_path(&dir.path().join("nope.json"));
        assert_eq!(settings, DictationSettings::default());
        assert_eq!(settings.cleanup, CleanupSource::Off);
        assert_eq!(settings.timeout_ms, 4_000);
        assert_eq!(settings.hotkey, "AltRight");
    }

    #[test]
    fn round_trips_the_agent_openrouter_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        let settings = DictationSettings {
            cleanup: CleanupSource::AgentOpenRouter {
                model: "google/gemini-3.8-flash".into(),
            },
            ..DictationSettings::default()
        };
        save_to_path(&path, &settings).unwrap();
        assert_eq!(load_from_path(&path), settings);
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["cleanup"]["kind"], "agent_open_router");
        let mut blank = settings.clone();
        blank.cleanup = CleanupSource::AgentOpenRouter { model: " ".into() };
        assert!(blank.validate().is_err());
    }

    #[test]
    fn round_trips_a_connection_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        let settings = DictationSettings {
            cleanup: CleanupSource::Connection {
                connection_id: "abc".into(),
                model: "google/gemini-3.8-flash".into(),
            },
            ..DictationSettings::default()
        };
        save_to_path(&path, &settings).unwrap();
        assert_eq!(load_from_path(&path), settings);
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["cleanup"]["kind"], "connection");
        assert_eq!(raw["cleanup"]["connectionId"], "abc");
    }

    #[test]
    fn invalid_or_tampered_files_fall_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        for body in [
            "not json",
            r#"{"version":1,"cleanup":{"kind":"off"},"timeoutMs":99999,"hotkey":"AltRight","language":"en","cloudCleanupEnabled":true}"#,
            r#"{"version":1,"cleanup":{"kind":"off"},"timeoutMs":4000,"hotkey":"Alt Right!","language":"en","cloudCleanupEnabled":true}"#,
            r#"{"version":7,"cleanup":{"kind":"off"},"timeoutMs":4000,"hotkey":"AltRight","language":"en","cloudCleanupEnabled":true}"#,
        ] {
            std::fs::write(&path, body).unwrap();
            assert_eq!(
                load_from_path(&path),
                DictationSettings::default(),
                "{body}"
            );
        }
    }

    #[test]
    fn legacy_file_migrates_without_losing_cleanup_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        std::fs::write(&path, r#"{"version":1,"cleanup":{"kind":"agent_open_router","model":"custom"},"timeoutMs":6000,"hotkey":"KeyA","language":"en","cloudCleanupEnabled":false}"#).unwrap();
        let saved = load_from_path(&path);
        assert_eq!(saved.version, 2);
        assert_eq!(saved.speech, SpeechSource::default());
        assert_eq!(saved.hotkey, "AltRight");
        assert_eq!(saved.timeout_ms, 6000);
        assert_eq!(
            saved.cleanup,
            CleanupSource::AgentOpenRouter {
                model: "custom".into()
            }
        );
        assert!(!saved.cloud_cleanup_enabled);
        save_to_path(&path, &saved).unwrap();
        assert_eq!(load_from_path(&path), saved);
    }

    #[test]
    fn save_rejects_conflicting_keys_and_unknown_model_ids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        for key in [
            "KeyA",
            "Space",
            "Enter",
            "Tab",
            "Escape",
            "ControlLeft",
            "MetaRight",
            "AltGraph",
            "F5",
            "F11",
            "F12",
            "F013",
        ] {
            let settings = DictationSettings {
                hotkey: key.into(),
                ..DictationSettings::default()
            };
            assert!(save_to_path(&path, &settings).is_err(), "{key}");
        }
        for key in ["AltRight", "AltLeft", "F8", "F9", "F13", "F24"] {
            let settings = DictationSettings {
                hotkey: key.into(),
                ..DictationSettings::default()
            };
            save_to_path(&path, &settings).unwrap();
        }
        let settings = DictationSettings {
            speech: SpeechSource::Local {
                model_id: "unknown".into(),
            },
            ..DictationSettings::default()
        };
        assert!(save_to_path(&path, &settings).is_err());
    }

    #[test]
    fn saving_invalid_settings_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let settings = DictationSettings {
            timeout_ms: 10,
            ..DictationSettings::default()
        };
        assert!(save_to_path(&dir.path().join(SETTINGS_FILE), &settings).is_err());
    }
}
