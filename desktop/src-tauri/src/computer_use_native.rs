//! The GUI app owns the native driver's TCC responsibility and lifetime.
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};
use std::{path::PathBuf, process::Child, sync::Mutex};
use tauri::{AppHandle, Manager};

#[derive(Default)]
pub(crate) struct NativeDriver(Mutex<Option<Running>>);
struct Running {
    child: Child,
    directory: PathBuf,
    #[cfg(target_os = "macos")]
    socket: PathBuf,
}

pub(crate) fn binary(app: &AppHandle) -> Option<PathBuf> {
    let path = app
        .path()
        .resource_dir()
        .ok()?
        .join("computer-use/native/cua-driver");
    if path.is_file() {
        return Some(path);
    }
    #[cfg(debug_assertions)]
    {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.cache/computer-use/runtime/native/cua-driver");
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

impl NativeDriver {
    #[cfg(target_os = "macos")]
    pub(crate) fn start(&self, app: &AppHandle) -> Result<(PathBuf, PathBuf), String> {
        use std::os::unix::{fs::DirBuilderExt, net::UnixStream};
        let binary = binary(app).ok_or("Native driver is not bundled in this build")?;
        let mut state = self.0.lock().map_err(|e| e.to_string())?;
        if let Some(running) = state.as_mut() {
            if running
                .child
                .try_wait()
                .map_err(|e| e.to_string())?
                .is_none()
            {
                return Ok((binary, running.socket.clone()));
            }
        }
        if let Some(old) = state.take() {
            std::fs::remove_dir_all(old.directory).map_err(|e| e.to_string())?;
        }
        // Short private path fits macOS sockaddr_un. Never use a shared, predictable socket.
        let directory = PathBuf::from("/tmp").join(format!("buzz-cua-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|e| e.to_string())?;
        let socket = directory.join("driver.sock");
        let mut command = Command::new(&binary);
        command.env_clear();
        for key in ["HOME", "PATH", "TMPDIR", "LANG"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .args([
                "serve",
                "--embedded",
                "--parent-liveness-stdio",
                "--no-permissions-gate",
                "--permission-mode",
                "standard",
                "--host-bundle-id",
                "xyz.block.buzz.app",
                "--socket",
            ])
            .arg(&socket)
            .env("CUA_DRIVER_EMBEDDED", "1")
            .env(
                "CUA_DRIVER_EMBEDDED_HOST_PID",
                std::process::id().to_string(),
            )
            .env("CUA_DRIVER_RS_TELEMETRY_ENABLED", "false")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let mut child = command
            .spawn()
            .map_err(|e| format!("start native driver: {e}"))?;
        for _ in 0..100 {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                std::fs::remove_dir_all(&directory).map_err(|e| e.to_string())?;
                return Err(format!("Native driver exited during startup: {status}"));
            }
            if UnixStream::connect(&socket).is_ok() {
                *state = Some(Running {
                    child,
                    directory,
                    socket: socket.clone(),
                });
                return Ok((binary, socket));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        child.kill().map_err(|e| e.to_string())?;
        child.wait().map_err(|e| e.to_string())?;
        std::fs::remove_dir_all(directory).map_err(|e| e.to_string())?;
        Err("Native driver did not become ready within five seconds".into())
    }

    #[cfg(not(target_os = "macos"))]
    pub(crate) fn start(&self, _app: &AppHandle) -> Result<(PathBuf, PathBuf), String> {
        Err("Native app control is currently supported only on macOS".into())
    }

    pub(crate) fn stop(&self) -> Result<(), String> {
        let mut state = self.0.lock().map_err(|e| e.to_string())?;
        if let Some(mut running) = state.take() {
            // EOF is the driver's supported parent-liveness shutdown contract.
            drop(running.child.stdin.take());
            for _ in 0..50 {
                if running
                    .child
                    .try_wait()
                    .map_err(|e| e.to_string())?
                    .is_some()
                {
                    std::fs::remove_dir_all(running.directory).map_err(|e| e.to_string())?;
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            running.child.kill().map_err(|e| e.to_string())?;
            running.child.wait().map_err(|e| e.to_string())?;
            std::fs::remove_dir_all(running.directory).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
