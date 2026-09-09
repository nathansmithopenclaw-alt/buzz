use super::probe;
use futures_util::{stream, StreamExt};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    net::Ipv4Addr,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
static SCANNING: AtomicBool = AtomicBool::new(false);
struct ScanGuard;
impl Drop for ScanGuard {
    fn drop(&mut self) {
        SCANNING.store(false, Ordering::Release);
    }
}
const PORTS: &[u16] = &[11434, 1234, 8000, 8010, 8080];
#[derive(Serialize)]
pub struct DiscoveredServer {
    pub name: String,
    pub base_url: String,
    pub models: Vec<String>,
    pub needs_key: bool,
}
#[derive(Serialize)]
pub struct DiscoveryResult {
    pub servers: Vec<DiscoveredServer>,
    pub notice: String,
}
fn subnet_hosts(ip: Ipv4Addr, mask: Ipv4Addr) -> Vec<Ipv4Addr> {
    if !ip.is_private() {
        return Vec::new();
    }
    // Never fan out over an entire enterprise /8 or /16. Search the attached /24
    // containing this device; service advertisements cover other local hosts.
    let mask = u32::from(mask) | 0xffff_ff00;
    let network = u32::from(ip) & mask;
    let broadcast = network | !mask;
    (network.saturating_add(1)..broadcast)
        .take(254)
        .map(Ipv4Addr::from)
        .collect()
}
fn advertised() -> Vec<(String, String)> {
    let Ok(daemon) = mdns_sd::ServiceDaemon::new() else {
        return Vec::new();
    };
    let mut receivers = Vec::new();
    for service in [
        "_ollama._tcp.local.",
        "_lmstudio._tcp.local.",
        "_openai._tcp.local.",
        "_buzz-model._tcp.local.",
    ] {
        if let Ok(receiver) = daemon.browse(service) {
            receivers.push(receiver);
        }
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut rows = Vec::new();
    while std::time::Instant::now() < deadline && rows.len() < 128 {
        for receiver in &receivers {
            if let Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) = receiver.try_recv() {
                for ip in info.get_addresses_v4() {
                    if !ip.is_private() && !ip.is_loopback() {
                        continue;
                    }
                    let name = info.get_hostname().trim_end_matches('.').to_string();
                    let path = info
                        .get_property_val_str("path")
                        .filter(|s| s.starts_with('/') && s.len() < 128)
                        .unwrap_or("/v1");
                    rows.push((format!("http://{ip}:{}{path}", info.get_port()), name));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = daemon.shutdown();
    rows
}
#[tauri::command]
pub async fn discover_model_servers(
    include_private_network: Option<bool>,
) -> Result<DiscoveryResult, String> {
    if SCANNING.swap(true, Ordering::AcqRel) {
        return Err("A model server search is already running".into());
    }
    let _guard = ScanGuard;
    let permission_denied = tokio::task::spawn_blocking(super::local_permission::denied)
        .await
        .map_err(|_| "Local network permission check stopped unexpectedly")?;
    let interfaces =
        if_addrs::get_if_addrs().map_err(|_| "Cannot inspect local network interfaces")?;
    let mut hosts = BTreeSet::from([Ipv4Addr::LOCALHOST]);
    let mut limited = false;
    for interface in interfaces {
        if interface.name.starts_with("utun")
            || interface.name.starts_with("tun")
            || interface.name.starts_with("tailscale")
        {
            continue;
        }
        if let if_addrs::IfAddr::V4(addr) = interface.addr {
            limited |= addr.ip.is_private() && u32::from(addr.netmask) < 0xffff_ff00;
            for ip in subnet_hosts(addr.ip, addr.netmask) {
                if hosts.len() < 509 {
                    hosts.insert(ip);
                } else {
                    limited = true;
                }
            }
        }
    }
    let mut targets: BTreeMap<String, String> = hosts
        .into_iter()
        .flat_map(|ip| {
            PORTS.iter().map(move |port| {
                (
                    format!("http://{ip}:{port}/v1"),
                    if ip.is_loopback() {
                        "This computer".to_string()
                    } else {
                        ip.to_string()
                    },
                )
            })
        })
        .collect();
    for (url, name) in tokio::task::spawn_blocking(advertised)
        .await
        .map_err(|_| "Service discovery stopped unexpectedly")?
    {
        targets.insert(url, name);
    }
    let mut private_notice = String::new();
    if include_private_network == Some(true) {
        match super::private_network::discover().await {
            Ok(peers) => {
                for (ip, name) in peers {
                    for port in PORTS {
                        targets.insert(format!("http://{ip}:{port}/v1"), name.clone());
                    }
                }
            }
            Err(error) => private_notice = error,
        }
    }
    let mut pending = stream::iter(targets)
        .map(|(url, name)| async move {
            let parsed = reqwest::Url::parse(&url).ok()?;
            let host = parsed.host_str()?;
            let port = parsed.port_or_known_default()?;
            tokio::time::timeout(
                Duration::from_millis(400),
                tokio::net::TcpStream::connect((host, port)),
            )
            .await
            .ok()?
            .ok()?;
            match probe::catalog(&url, None, 2).await {
                Ok(models) => Some(DiscoveredServer {
                    name,
                    base_url: url,
                    models,
                    needs_key: false,
                }),
                Err(error) if error.contains("HTTP 401") || error.contains("HTTP 403") => {
                    Some(DiscoveredServer {
                        name,
                        base_url: url,
                        models: Vec::new(),
                        needs_key: true,
                    })
                }
                _ => None,
            }
        })
        .buffer_unordered(64);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    let mut servers = Vec::new();
    loop {
        match tokio::time::timeout_at(deadline, pending.next()).await {
            Ok(Some(Some(server))) => servers.push(server),
            Ok(Some(None)) => {}
            Ok(None) => break,
            Err(_) => {
                limited = true;
                break;
            }
        }
    }
    servers.sort_by(|a, b| a.name.cmp(&b.name).then(a.base_url.cmp(&b.base_url)));
    let mut notice = if !private_notice.is_empty() {
        private_notice
    } else if limited {
        "Searched this computer and nearby local addresses. Larger networks, VPNs, and custom ports may need manual setup.".into()
    } else {
        "Searched this computer, local service advertisements, and common model ports. Private networks and custom ports may need manual setup.".into()
    };
    if permission_denied {
        notice.push_str(" macOS denied local network access. Enable Buzz in System Settings → Privacy & Security → Local Network, then search again.");
    } else if cfg!(target_os = "macos") && servers.is_empty() {
        notice.push_str(" On macOS, allow Buzz's Local Network request. If access was previously denied, check System Settings → Privacy & Security → Local Network, then search again.");
    }
    Ok(DiscoveryResult { servers, notice })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "explicit local network acceptance; performs bounded catalog discovery"]
    async fn live_model_server_discovery() {
        let result = discover_model_servers(Some(true)).await.unwrap();
        println!(
            "MODEL_DISCOVERY_RESULT={}",
            serde_json::to_string(&result).unwrap()
        );
    }
    #[test]
    fn scan_never_expands_public_or_large_networks() {
        assert!(
            subnet_hosts(Ipv4Addr::new(8, 8, 8, 8), Ipv4Addr::new(255, 255, 255, 0)).is_empty()
        );
        let hosts = subnet_hosts(Ipv4Addr::new(10, 2, 3, 4), Ipv4Addr::new(255, 0, 0, 0));
        assert_eq!(hosts.len(), 254);
        assert!(hosts.iter().all(|ip| ip.octets()[..3] == [10, 2, 3]));
    }
}
