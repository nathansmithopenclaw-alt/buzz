//! Private, generation-bound Desktop admission barrier. No credentials cross this IPC.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct Request {
    nonce: String,
    token: String,
    action: String,
}
#[derive(Serialize)]
struct Response<'a> {
    nonce: &'a str,
    token: &'a str,
    status: &'a str,
}

/// A barrier belongs to one harness generation and survives pool replacement.
pub struct AccountControl {
    directory: Option<PathBuf>,
    nonce: String,
    held_by: Option<String>,
    resume_auth: bool,
    last_token_action: Option<(String, String)>,
}
impl AccountControl {
    /// Construct only from trusted Desktop child environment, never relay payloads.
    pub fn from_env() -> Self {
        Self::new(
            std::env::var_os("BUZZ_ACP_ACCOUNT_CONTROL_DIR").map(PathBuf::from),
            std::env::var("BUZZ_MANAGED_AGENT_START_NONCE").unwrap_or_default(),
        )
    }
    fn new(directory: Option<PathBuf>, nonce: String) -> Self {
        Self {
            directory,
            nonce,
            held_by: None,
            resume_auth: false,
            last_token_action: None,
        }
    }
    /// Inspect only bounded non-secret IPC; avoids draining adapters on every tick.
    pub fn has_pending_request(&self) -> bool {
        let Some(directory) = &self.directory else {
            return false;
        };
        let Ok(Some(raw)) = read_bounded(&directory.join("request.json")) else {
            return false;
        };
        let Ok(request) = serde_json::from_slice::<Request>(&raw) else {
            return false;
        };
        request.nonce == self.nonce
            && self.last_token_action.as_ref() != Some(&(request.token, request.action))
    }

    /// Read one bounded command. Call in the scheduling loop before admission;
    /// `busy` includes checked-out workers and an in-flight pool wake.
    pub fn poll(&mut self, busy: bool) -> Result<(), String> {
        let Some(dir) = &self.directory else {
            return Ok(());
        };
        let path = dir.join("request.json");
        let raw = match read_bounded(&path) {
            Ok(Some(raw)) => raw,
            Ok(None) => return Ok(()),
            Err(e) => return Err(e),
        };
        let request: Request = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
        if request.nonce != self.nonce
            || request.token.len() != 32
            || !request.token.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("invalid subscription switch request generation".into());
        }
        let key = (request.token.clone(), request.action.clone());
        if self.last_token_action.as_ref() == Some(&key) {
            return Ok(());
        }
        let status = match request.action.as_str() {
            "quiesce" if self.held_by.as_ref().is_some_and(|v| v != &request.token) => "busy",
            "quiesce" if busy => "busy",
            "quiesce" => {
                self.held_by = Some(request.token.clone());
                "quiesced"
            }
            "release" if self.held_by.as_ref().is_none_or(|v| v == &request.token) => {
                self.held_by = None;
                "released"
            }
            "release" => "busy",
            "resume_auth" => {
                self.resume_auth = true;
                "resume_requested"
            }
            _ => return Err("unknown subscription switch action".into()),
        };
        let response = serde_json::to_vec(&Response {
            nonce: &self.nonce,
            token: &request.token,
            status,
        })
        .map_err(|e| e.to_string())?;
        let tmp = dir.join("response.tmp");
        std::fs::write(&tmp, response).map_err(|e| e.to_string())?;
        std::fs::rename(tmp, dir.join("response.json")).map_err(|e| e.to_string())?;
        self.last_token_action = Some(key);
        Ok(())
    }
    /// Whether all dispatch and pool wake paths must remain paused.
    /// Consume an explicit Desktop recovery request.
    pub fn take_resume_auth(&mut self) -> bool {
        std::mem::take(&mut self.resume_auth)
    }
    /// Publish the actual durable resume result after the queue commit.
    pub fn finish_resume_auth(&self, result: Result<(), String>) -> Result<(), String> {
        let Some(dir) = &self.directory else {
            return Ok(());
        };
        let Some((token, action)) = &self.last_token_action else {
            return Ok(());
        };
        if action != "resume_auth" {
            return Ok(());
        }
        let status = if result.is_ok() {
            "resumed"
        } else {
            "resume_failed"
        };
        let bytes = serde_json::to_vec(&Response {
            nonce: &self.nonce,
            token,
            status,
        })
        .map_err(|e| e.to_string())?;
        let tmp = dir.join("response.tmp");
        std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(tmp, dir.join("response.json")).map_err(|e| e.to_string())
    }
    pub fn enabled(&self) -> bool {
        self.directory.is_some()
    }

    pub fn is_quiesced(&self) -> bool {
        self.held_by.is_some()
    }
}
fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !meta.is_file() || meta.len() > 2048 {
        return Err("invalid subscription switch request file".into());
    }
    std::fs::read(path).map(Some).map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn busy_stale_and_release_are_fenced() {
        let dir = tempfile::tempdir().unwrap();
        let mut gate = AccountControl::new(Some(dir.path().into()), "generation".into());
        let token = "a".repeat(32);
        let write = |nonce: &str, action: &str| {
            std::fs::write(
                dir.path().join("request.json"),
                serde_json::json!({"nonce":nonce,"token":token,"action":action}).to_string(),
            )
            .unwrap()
        };
        write("stale", "quiesce");
        assert!(gate.poll(false).is_err());
        assert!(!gate.is_quiesced());
        write("generation", "quiesce");
        gate.poll(true).unwrap();
        assert!(!gate.is_quiesced());
        // Busy replies do not later become successful without a fresh owner request.
        gate.poll(false).unwrap();
        assert!(!gate.is_quiesced());
        write("generation", "release");
        gate.poll(false).unwrap();
        write("generation", "quiesce");
        gate.poll(false).unwrap();
        assert!(gate.is_quiesced());
        write("generation", "release");
        gate.poll(false).unwrap();
        assert!(!gate.is_quiesced());
    }
}
