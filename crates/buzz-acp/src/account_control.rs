//! Private, generation-bound Desktop admission barrier. No credentials cross this IPC.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct Request {
    nonce: String,
    token: String,
    action: String,
    #[serde(default)]
    request_id: String,
}
#[derive(Serialize)]
struct Response<'a> {
    nonce: &'a str,
    token: &'a str,
    status: &'a str,
    request_id: &'a str,
}

/// A barrier belongs to one harness generation and survives pool replacement.
pub struct AccountControl {
    directory: Option<PathBuf>,
    nonce: String,
    held_by: Option<String>,
    resume_auth: bool,
    pause_requested: bool,
    last_token_action: Option<(String, String)>,
    last_request_id: String,
    last_seen_request: Option<(String, String, String)>,
}
impl AccountControl {
    /// Construct only from trusted Desktop child environment, never relay payloads.
    pub fn from_env() -> Self {
        let mut control = Self::new(
            std::env::var_os("BUZZ_ACP_ACCOUNT_CONTROL_DIR").map(PathBuf::from),
            std::env::var("BUZZ_MANAGED_AGENT_START_NONCE").unwrap_or_default(),
        );
        if std::env::var("BUZZ_ACP_ACCOUNT_SWITCH_RESUME").as_deref() == Ok("0") {
            control.held_by = std::env::var("BUZZ_ACP_ACCOUNT_SWITCH_ID").ok();
        }
        control
    }
    fn new(directory: Option<PathBuf>, nonce: String) -> Self {
        Self {
            directory,
            nonce,
            held_by: None,
            resume_auth: false,
            pause_requested: false,
            last_token_action: None,
            last_request_id: String::new(),
            last_seen_request: None,
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
            && self.last_seen_request.as_ref()
                != Some(&(request.token, request.action, request.request_id))
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
        let seen = (
            request.token.clone(),
            request.action.clone(),
            request.request_id.clone(),
        );
        if self.last_seen_request.as_ref() == Some(&seen) {
            return Ok(());
        }
        if request.action == "probe" {
            // A probe cannot replace the correlation of a pause/resume that
            // is still waiting for adapter cleanup or a durable queue commit.
            let response = serde_json::to_vec(&Response {
                nonce: &self.nonce,
                token: &request.token,
                status: "listening",
                request_id: &request.request_id,
            })
            .map_err(|e| e.to_string())?;
            write_response(dir, &response)?;
            self.last_seen_request = Some(seen);
            return Ok(());
        }
        let key = (request.token.clone(), request.action.clone());
        self.last_request_id = request.request_id;
        let status = match request.action.as_str() {
            "pause" if busy && self.held_by.as_ref().is_some_and(|v| v != &request.token) => "busy",
            "pause" => {
                self.held_by = Some(request.token.clone());
                self.pause_requested = true;
                "pausing"
            }
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
            "resume_auth" if self.held_by.as_ref().is_some_and(|v| v != &request.token) => "busy",
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
            request_id: &self.last_request_id,
        })
        .map_err(|e| e.to_string())?;
        write_response(dir, &response)?;
        self.last_token_action = Some(key);
        self.last_seen_request = Some(seen);
        Ok(())
    }
    /// Whether all dispatch and pool wake paths must remain paused.
    /// Consume an explicit Desktop recovery request.
    pub fn take_resume_auth(&mut self) -> bool {
        std::mem::take(&mut self.resume_auth)
    }
    /// Publish the actual durable resume result after the queue commit.
    pub fn finish_resume_auth(&mut self, result: Result<(), String>) -> Result<(), String> {
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
            self.held_by = None;
            "resumed"
        } else {
            "resume_failed"
        };
        let bytes = serde_json::to_vec(&Response {
            nonce: &self.nonce,
            token,
            status,
            request_id: &self.last_request_id,
        })
        .map_err(|e| e.to_string())?;
        write_response(dir, &bytes)
    }
    /// Consume the owner-authorized active-turn pause once.
    pub fn take_pause(&mut self) -> Option<String> {
        if std::mem::take(&mut self.pause_requested) {
            self.held_by.clone()
        } else {
            None
        }
    }

    /// Acknowledge only after task cleanup and the durable handoff have completed.
    pub fn finish_pause(&self, success: bool) -> Result<(), String> {
        let Some(dir) = &self.directory else {
            return Ok(());
        };
        let Some(token) = &self.held_by else {
            return Ok(());
        };
        let bytes = serde_json::to_vec(&Response {
            nonce: &self.nonce,
            token,
            status: if success { "paused" } else { "pause_failed" },
            request_id: &self.last_request_id,
        })
        .map_err(|e| e.to_string())?;
        write_response(dir, &bytes)
    }

    pub fn enabled(&self) -> bool {
        self.directory.is_some()
    }

    pub fn is_quiesced(&self) -> bool {
        self.held_by.is_some()
    }
}
fn write_response(dir: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let tmp = dir.join(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp).map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dir.join("response.json")).map_err(|e| e.to_string())?;
    std::fs::File::open(dir)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())
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
    fn readiness_probe_is_generation_bound_and_preserves_a_busy_hold() {
        let dir = tempfile::tempdir().unwrap();
        let mut gate = AccountControl::new(Some(dir.path().into()), "generation".into());
        gate.held_by = Some("a".repeat(32));
        for nonce in ["stale", "generation"] {
            std::fs::write(
                dir.path().join("request.json"),
                serde_json::json!({
                    "nonce":nonce, "token":"b".repeat(32), "action":"probe", "request_id":"probe-1"
                })
                .to_string(),
            )
            .unwrap();
            if nonce == "stale" {
                assert!(gate.poll(true).is_err());
            } else {
                gate.poll(true).unwrap();
            }
        }
        let response: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("response.json")).unwrap())
                .unwrap();
        assert_eq!(response["status"], "listening");
        assert_eq!(response["request_id"], "probe-1");
        assert!(gate.is_quiesced());
        assert!(!gate.take_resume_auth());
        assert!(gate.take_pause().is_none());
    }

    #[test]
    fn readiness_probe_cannot_replace_an_in_progress_pause_acknowledgment() {
        let dir = tempfile::tempdir().unwrap();
        let mut gate = AccountControl::new(Some(dir.path().into()), "generation".into());
        for (action, request_id) in [("pause", "pause-1"), ("probe", "probe-1")] {
            std::fs::write(dir.path().join("request.json"), serde_json::json!({
                "nonce":"generation", "token":"a".repeat(32), "action":action, "request_id":request_id
            }).to_string()).unwrap();
            assert!(gate.has_pending_request());
            gate.poll(true).unwrap();
            assert!(!gate.has_pending_request());
        }
        assert_eq!(gate.take_pause(), Some("a".repeat(32)));
        gate.finish_pause(true).unwrap();
        let response: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("response.json")).unwrap())
                .unwrap();
        assert_eq!(response["status"], "paused");
        assert_eq!(response["request_id"], "pause-1");
        assert!(gate.is_quiesced());
    }
    #[test]
    fn active_pause_requires_durable_ack_and_explicit_retry() {
        let dir = tempfile::tempdir().unwrap();
        let mut gate = AccountControl::new(Some(dir.path().into()), "generation".into());
        let token = "a".repeat(32);
        let write = |request_id: &str| {
            std::fs::write(dir.path().join("request.json"),
            serde_json::json!({"nonce":"generation","token":token,"action":"pause","request_id":request_id}).to_string()).unwrap()
        };
        write("first");
        gate.poll(true).unwrap();
        assert!(gate.is_quiesced());
        assert_eq!(gate.take_pause(), Some(token.clone()));
        gate.finish_pause(false).unwrap();
        gate.poll(false).unwrap();
        assert!(
            gate.take_pause().is_none(),
            "failed pause cannot retry itself"
        );
        write("retry");
        gate.poll(false).unwrap();
        assert_eq!(gate.take_pause(), Some(token));
        gate.finish_pause(true).unwrap();
        let receipt: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("response.json")).unwrap())
                .unwrap();
        assert_eq!(receipt["status"], "paused");
        assert_eq!(receipt["request_id"], "retry");
        assert!(
            gate.is_quiesced(),
            "acknowledging pause cannot release admissions"
        );
    }

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
