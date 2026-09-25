//! Run on Windows CI: the adapter owns only the launcher PID, so killing that
//! PID must terminate the final MCP server through its kill-on-close Job.
#![cfg(windows)]

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn windows_mcp_fixture() {
    let Ok(receipt) = std::env::var("BUZZ_TEST_MCP_RECEIPT") else {
        return;
    };
    assert_eq!(std::env::var("BUZZ_PRIVATE_KEY").unwrap(), "synthetic-key");
    assert_eq!(std::env::var("BUZZ_AUTH_TAG").unwrap(), "synthetic-auth");
    std::fs::write(receipt, std::process::id().to_string()).unwrap();
    // Ignore stdin closure like a hung MCP server. Bound the fixture even if
    // containment regresses, so an unsuccessful test cannot leave it running.
    std::thread::sleep(Duration::from_secs(15));
}

#[test]
fn killing_windows_launcher_terminates_credential_bearing_mcp_child() {
    let dir = tempfile::tempdir().unwrap();
    let receipt = dir.path().join("child-pid");
    let mut file = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
    serde_json::to_writer(
        &mut file,
        &serde_json::json!({
            "name":"fixture", "command":std::env::current_exe().unwrap(),
            "args":["--exact","windows_mcp_fixture","--nocapture"],
            "env":[
                {"name":"BUZZ_TEST_MCP_RECEIPT","value":receipt},
                {"name":"BUZZ_PRIVATE_KEY","value":"synthetic-key"},
                {"name":"BUZZ_AUTH_TAG","value":"synthetic-auth"}
            ]
        }),
    )
    .unwrap();
    file.flush().unwrap();
    let mut launcher = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_buzz-acp"))
            .arg("mcp-launch")
            .arg(file.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !receipt.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let pid: u32 = std::fs::read_to_string(&receipt).unwrap().parse().unwrap();
    launcher.0.kill().unwrap();
    launcher.0.wait().unwrap();
    let check = format!("@(Get-Process -Id {pid} -ErrorAction SilentlyContinue).Count");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        // Inspect only this fixture-owned PID and print a count, never argv/env.
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", &check])
            .output()
            .unwrap();
        if String::from_utf8_lossy(&output.stdout).trim() == "0" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "MCP fixture survived launcher termination"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
