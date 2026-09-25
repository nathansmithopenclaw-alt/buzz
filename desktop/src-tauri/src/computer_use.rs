//! Host-owned computer-use discovery and human browser inspection.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Default, Deserialize, Serialize)]
struct Settings {
    enabled: bool,
    #[serde(default)]
    native_enabled: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComputerStatus {
    enabled: bool,
    dependency_ready: bool,
    detail: String,
    native_enabled: bool,
    native_ready: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserReceipt {
    version: u32,
    id: String,
    owner: String,
    label: Option<String>,
    pid: u32,
    created: String,
    viewer_url: String,
}

fn root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|p| p.join("computer-use"))
        .map_err(|e| e.to_string())
}

fn settings(app: &AppHandle) -> Result<Settings, String> {
    match std::fs::read(root(app)?.join("settings.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(e.to_string()),
    }
}

fn service_script(app: &AppHandle) -> Option<PathBuf> {
    let packaged = app
        .path()
        .resource_dir()
        .ok()?
        .join("computer-use/src/main.mjs");
    if packaged.is_file() {
        return Some(packaged);
    }
    #[cfg(debug_assertions)]
    {
        let source =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../computer-use/src/main.mjs");
        if source.is_file() {
            return Some(source);
        }
    }
    None
}

#[tauri::command]
pub(crate) fn computer_use_status(app: AppHandle) -> Result<ComputerStatus, String> {
    let config = settings(&app)?;
    let enabled = config.enabled;
    let dependency_ready =
        service_script(&app).is_some() && crate::managed_agents::resolve_command("node").is_some();
    Ok(ComputerStatus { enabled, dependency_ready, native_enabled: config.native_enabled, native_ready: cfg!(target_os = "macos") && crate::computer_use_native::binary(&app).is_some(), detail: if !dependency_ready {
        "Browser runtime is missing. Install a Buzz build containing the computer-use service and Node runtime."
    } else if enabled {
        "Browser tools are enabled. Restart existing agents to discover them. Native app control has a separate opt-in and macOS permission check."
    } else {
        "Enable local browser tools for new agent sessions. Each session gets separate browser storage."
    }.to_string() })
}

#[tauri::command]
pub(crate) fn set_computer_use_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    if enabled
        && (service_script(&app).is_none()
            || crate::managed_agents::resolve_command("node").is_none())
    {
        return Err("Computer-use runtime dependencies are missing".into());
    }
    let directory = root(&app)?;
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let mut config = settings(&app)?;
    config.enabled = enabled;
    let bytes = serde_json::to_vec(&config).map_err(|e| e.to_string())?;
    crate::managed_agents::atomic_write_json_restricted(&directory.join("settings.json"), &bytes)
}

#[tauri::command]
pub(crate) fn set_computer_native_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    if enabled && crate::computer_use_native::binary(&app).is_none() {
        return Err("Native driver is not bundled in this build".into());
    }
    let mut config = settings(&app)?;
    config.native_enabled = enabled;
    let directory = root(&app)?;
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec(&config).map_err(|e| e.to_string())?;
    crate::managed_agents::atomic_write_json_restricted(&directory.join("settings.json"), &bytes)?;
    if !enabled {
        app.state::<crate::computer_use_native::NativeDriver>()
            .stop()?;
    }
    Ok(())
}

fn valid_viewer_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.port().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url
            .fragment()
            .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

#[tauri::command]
pub(crate) fn list_computer_browser_sessions(
    app: AppHandle,
) -> Result<Vec<BrowserReceipt>, String> {
    let directory = root(&app)?.join("sessions");
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let mut results = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let metadata = entry.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.len() > 8192 {
            continue;
        }
        let bytes = std::fs::read(entry.path()).map_err(|e| e.to_string())?;
        let Ok(receipt) = serde_json::from_slice::<BrowserReceipt>(&bytes) else {
            continue;
        };
        if !crate::managed_agents::process_is_running(receipt.pid) {
            // Unique receipt paths belong to one process and are never reused.
            if let Err(error) = std::fs::remove_file(entry.path()) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(error.to_string());
                }
            }
            continue;
        }
        if receipt.version == 1 && valid_viewer_url(&receipt.viewer_url) {
            results.push(receipt);
        }
        if results.len() >= 128 {
            break;
        }
    }

    results.sort_by(|a, b| a.created.cmp(&b.created));
    Ok(results)
}

pub(crate) fn clear_runtime_env(command: &mut std::process::Command) {
    // Prevent ambient values from enabling a capability the owner disabled in Buzz.
    for key in [
        "BUZZ_COMPUTER_NODE",
        "BUZZ_COMPUTER_SCRIPT",
        "BUZZ_COMPUTER_REGISTRY",
        "BUZZ_COMPUTER_OWNER",
        "BUZZ_COMPUTER_LABEL",
        "BUZZ_NATIVE_DRIVER",
        "BUZZ_NATIVE_SOCKET",
    ] {
        command.env_remove(key);
    }
}

/// Inject host-local service paths at the managed runtime boundary, never relay credentials.
pub(crate) fn configure_runtime(
    app: &AppHandle,
    command: &mut std::process::Command,
    owner: &str,
    label: &str,
) -> Result<(), String> {
    clear_runtime_env(command);
    let config = settings(app)?;
    if !config.enabled {
        return Ok(());
    }
    let script = service_script(app).ok_or("Computer-use service is enabled but missing")?;
    let node = crate::managed_agents::resolve_command("node")
        .ok_or("Computer-use service requires Node")?;
    command.env("BUZZ_COMPUTER_NODE", node);
    command.env("BUZZ_COMPUTER_SCRIPT", script);
    command.env("BUZZ_COMPUTER_REGISTRY", root(app)?.join("sessions"));
    command.env("BUZZ_COMPUTER_OWNER", owner);
    command.env("BUZZ_COMPUTER_LABEL", label);
    if config.native_enabled {
        let (driver, socket) = app
            .state::<crate::computer_use_native::NativeDriver>()
            .start(app)?;
        command.env("BUZZ_NATIVE_DRIVER", driver);
        command.env("BUZZ_NATIVE_SOCKET", socket);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn viewer_urls_cannot_redirect_the_human_control_token() {
        let token = "a".repeat(64);
        assert!(valid_viewer_url(&format!("http://127.0.0.1:4567/#{token}")));
        for base in [
            "https://evil.example:4567/",
            "http://127.0.0.1.evil.example:4567/",
            "http://user@127.0.0.1:4567/",
            "http://127.0.0.1:4567/other",
            "http://127.0.0.1:4567/?next=evil",
            "http://127.0.0.1/",
        ] {
            assert!(!valid_viewer_url(&format!("{base}#{token}")), "{base}");
        }
    }
}
