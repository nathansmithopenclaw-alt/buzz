//! Optional discovery of known Tailscale peers; never scans an overlay CIDR.
use std::{net::IpAddr, process::Stdio, time::Duration};
use tokio::io::AsyncReadExt;
fn peers(value: &serde_json::Value) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    if let Some(peers) = value.get("Peer").and_then(|v| v.as_object()) {
        for peer in peers.values().take(128) {
            if peer.get("Online").and_then(|v| v.as_bool()) != Some(true) {
                continue;
            }
            let name = peer
                .get("HostName")
                .and_then(|v| v.as_str())
                .unwrap_or("Private network server");
            if let Some(ips) = peer.get("TailscaleIPs").and_then(|v| v.as_array()) {
                for raw in ips.iter().filter_map(|v| v.as_str()) {
                    if let Ok(IpAddr::V4(ip)) = raw.parse::<IpAddr>() {
                        let octets = ip.octets();
                        if octets[0] == 100 && (64..128).contains(&octets[1]) {
                            rows.push((ip.to_string(), name.chars().take(100).collect()));
                        }
                    }
                }
            }
        }
    }
    rows
}
pub(super) async fn discover() -> Result<Vec<(String, String)>, String> {
    let command = crate::managed_agents::resolve_command("tailscale")
        .or_else(|| {
            let path =
                std::path::PathBuf::from("/Applications/Tailscale.app/Contents/MacOS/Tailscale");
            path.is_file().then_some(path)
        })
        .ok_or("Tailscale is not installed. Add a private-network server manually.")?;
    let mut child = tokio::process::Command::new(command)
        .args(["status", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "Cannot read the private network")?;
    let output = child
        .stdout
        .take()
        .ok_or("Cannot read private network status")?;
    let mut bytes = Vec::new();
    let read = tokio::time::timeout(
        Duration::from_secs(3),
        output.take(512 * 1024 + 1).read_to_end(&mut bytes),
    )
    .await;
    if !matches!(read, Ok(Ok(_))) || bytes.len() > 512 * 1024 {
        let _ = child.kill().await;
        return Err("Private network status timed out or exceeded its limit".into());
    }
    let status = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
    if !matches!(status, Ok(Ok(status)) if status.success()) {
        let _ = child.kill().await;
        return Err(
            "Private network is unavailable. Check Tailscale or add a server manually.".into(),
        );
    }
    let value =
        serde_json::from_slice(&bytes).map_err(|_| "Private network returned invalid status")?;
    Ok(peers(&value))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_online_overlay_peers_are_considered() {
        let value = serde_json::json!({"Peer":{
            "a":{"HostName":"Spark 1","Online":true,"TailscaleIPs":["100.80.202.75","8.8.8.8","::1"]},
            "b":{"HostName":"Offline","Online":false,"TailscaleIPs":["100.80.1.2"]}
        }});
        assert_eq!(
            peers(&value),
            vec![("100.80.202.75".into(), "Spark 1".into())]
        );
    }
}
