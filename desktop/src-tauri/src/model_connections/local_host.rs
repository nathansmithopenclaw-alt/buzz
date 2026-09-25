//! Literal-host classification for display grouping, never authorization.

use std::net::IpAddr;

/// Recognizes local/private literal hosts, including carrier-grade NAT overlays.
pub(crate) fn is_local_model_host(base_url: &str) -> bool {
    let Ok(url) = url::Url::parse(base_url) else {
        return false;
    };
    match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => local_ip(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => local_ip(IpAddr::V6(ip)),
        None => false,
    }
}

fn local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip
                    .to_ipv4_mapped()
                    .is_some_and(|ip| local_ip(IpAddr::V4(ip)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn groups_private_literal_hosts_including_tailscale() {
        for host in [
            "localhost",
            "127.0.0.1",
            "10.1.2.3",
            "172.16.1.1",
            "192.168.1.1",
            "169.254.2.1",
            "100.64.0.0",
            "100.80.202.75",
            "100.127.255.255",
            "[::1]",
            "[fd00::1]",
            "[fe80::1]",
        ] {
            assert!(
                is_local_model_host(&format!("http://{host}:8010/v1")),
                "{host}"
            );
        }
        for host in [
            "100.63.255.255",
            "100.128.0.0",
            "172.32.0.1",
            "8.8.8.8",
            "localhost.example.com",
            "example.com",
            "[2001:4860:4860::8888]",
        ] {
            assert!(
                !is_local_model_host(&format!("https://{host}/v1")),
                "{host}"
            );
        }
    }
}
