//! Bounded ACP status probe. The provider owns credentials; only metadata is read.
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

pub(super) fn status(
    executable: &Path,
    home: &Path,
    cancelled: &Arc<AtomicBool>,
) -> Result<(), String> {
    let timeout = Duration::from_secs(60);
    let mut command =
        super::adapters::isolated_command(executable, "grok", home, &["agent", "stdio"], timeout);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Could not start Grok account check")?;
    let pid = child.id();
    let result = (|| {
        let stdout = child
            .stdout
            .take()
            .ok_or("Grok account output unavailable")?;
        let mut stdin = child.stdin.take().ok_or("Grok account input unavailable")?;
        let (tx, rx) = mpsc::sync_channel(16);
        std::thread::spawn(move || {
            // Includes non-JSON diagnostics: never log or expose provider output.
            for line in BufReader::new(stdout.take(1024 * 1024)).lines() {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let deadline = Instant::now() + timeout;
        let mut request = |id: u64, method: &str, params: Value| -> Result<Value, String> {
            writeln!(
                stdin,
                "{}",
                json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
            )
            .map_err(|_| "Could not send Grok account check")?;
            stdin
                .flush()
                .map_err(|_| "Could not flush Grok account check")?;
            loop {
                if cancelled.load(Ordering::SeqCst) {
                    return Err("Login cancelled".into());
                }
                if Instant::now() >= deadline {
                    return Err("Grok account check timed out; reconnect to retry".into());
                }
                match rx.recv_timeout(Duration::from_millis(25)) {
                    Ok(Ok(line)) => {
                        let Ok(value) = serde_json::from_str::<Value>(&line) else {
                            continue;
                        };
                        if value["id"].as_u64() != Some(id) {
                            continue;
                        }
                        if value.get("error").is_some() {
                            return Err("Grok account check failed; reconnect or check the installed Grok version".into());
                        }
                        return value
                            .get("result")
                            .cloned()
                            .ok_or_else(|| "Invalid Grok account check response".into());
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    _ => return Err("Grok account check ended before verification".into()),
                }
            }
        };
        let initialized = request(
            1,
            "initialize",
            json!({"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"buzz-account-check","version":"1"}}),
        )?;
        if initialized["protocolVersion"].as_u64() != Some(1) {
            return Err("Unsupported Grok account protocol".into());
        }
        request(2, "authenticate", json!({"methodId":"cached_token"}))?;
        let info = request(3, "_x.ai/auth/info", json!({}))?;
        let status = request(4, "_x.ai/auth/check_subscription", json!({}))?;
        buzz_core_pkg::grok_subscription::verify_status(&info, &status)
    })();
    let cleanup = super::process::kill_group(pid);
    if cleanup.is_err() {
        let _ = child.kill();
    }
    let reap = child
        .wait()
        .map_err(|_| "Could not reap Grok account check");
    cleanup?;
    reap?;
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn provider(root: &Path, status: Value) -> std::path::PathBuf {
        let path = root.join("grok-fixture");
        let script = format!(
            r##"#!/usr/bin/python3
import sys,json,os
assert os.path.realpath(os.environ['GROK_HOME']) == os.getcwd()
assert 'XAI_API_KEY' not in os.environ
for line in sys.stdin:
 r=json.loads(line)
 if r['method']=='initialize': result={{'protocolVersion':1}}
 elif r['method']=='authenticate':
  assert r['params']['methodId']=='cached_token'
  result={{}}
 elif r['method']=='_x.ai/auth/info': result={{'methodId':'cached_token','email':'test@example.invalid','userBlockedReason':None,'teamBlockedReasons':[]}}
 elif r['method']=='_x.ai/auth/check_subscription': result=json.loads({status:?})
 else: raise Exception('unexpected method')
 print(json.dumps({{'jsonrpc':'2.0','id':r['id'],'result':result}}),flush=True)
"##,
            status = status.to_string()
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
    #[test]
    fn provider_owned_status_protocol_accepts_only_subscription() {
        let root = tempfile::tempdir().unwrap();
        let valid = json!({"authenticated":true,"meta":{"auth_mode":"Oidc","subscription_tier":"SuperGrok","gate":null}});
        let executable = provider(root.path(), valid);
        let cancelled = Arc::new(AtomicBool::new(false));
        status(&executable, root.path(), &cancelled).unwrap();
        let executable = provider(root.path(), json!({"authenticated":false,"meta":null}));
        assert!(status(&executable, root.path(), &cancelled)
            .unwrap_err()
            .contains("subscription authentication"));
        cancelled.store(true, Ordering::SeqCst);
        assert!(status(&executable, root.path(), &cancelled)
            .unwrap_err()
            .contains("cancelled"));
    }
}
