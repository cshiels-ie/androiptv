//! Local network helpers.

use std::net::IpAddr;

/// Virtual/software adapters that are never the machine's LAN address:
/// docker bridges, veth pairs, VPNs, VM networking, ... (matched as
/// prefixes, case-insensitive).
const VIRTUAL_PREFIXES: &[&str] = &[
    "docker", "br-", "veth", "virbr", "vmnet", "vbox", "tun", "tap", "utun",
    "wg", "ppp", "zt", "tailscale", "vpn",
];

/// All usable local IPv4 addresses (non-loopback, non-link-local, IPv4
/// only), as strings. Real interfaces (wlan/eth/en/...) come first; any
/// remaining address (VPN, VM, ...) is appended afterwards so the first
/// entry — which the server URL is built from — is almost always the one
/// reachable on the user's Wi-Fi. Returns an empty vec if the interface
/// list can't be read (never panics).
pub fn local_ips() -> Vec<String> {
    let Ok(interfaces) = local_ip_address::list_afinet_netifas() else {
        return Vec::new();
    };
    relevant_ips(interfaces)
}

/// Pure filtering/sorting of (name, ip) pairs — separated out so the
/// ordering rules are unit-testable without real network state.
fn relevant_ips(interfaces: Vec<(String, IpAddr)>) -> Vec<String> {
    let mut physical: Vec<String> = Vec::new();
    let mut virtual_ips: Vec<String> = Vec::new();
    for (name, ip) in interfaces {
        let valid = match ip {
            // IPv4 only; drop loopback, link-local and unspecified.
            IpAddr::V4(v4) => !v4.is_loopback() && !v4.is_link_local() && !v4.is_unspecified(),
            IpAddr::V6(_) => false,
        };
        if !valid {
            continue;
        }
        let lower = name.to_lowercase();
        if VIRTUAL_PREFIXES.iter().any(|p| lower.starts_with(p)) {
            virtual_ips.push(ip.to_string());
        } else {
            physical.push(ip.to_string());
        }
    }
    physical.sort();
    physical.dedup();
    virtual_ips.sort();
    virtual_ips.dedup();
    physical.append(&mut virtual_ips);
    physical
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn v4(octets: [u8; 4]) -> IpAddr {
        IpAddr::V4(Ipv4Addr::from(octets))
    }

    fn iface(name: &str, ip: IpAddr) -> (String, IpAddr) {
        (name.to_string(), ip)
    }

    #[test]
    fn physical_interfaces_come_first() {
        let ips = relevant_ips(vec![
            iface("wlan0", v4([192, 168, 1, 50])),
            iface("docker0", v4([172, 17, 0, 1])),
            iface("eth0", v4([192, 168, 1, 40])),
        ]);
        assert_eq!(ips, vec!["192.168.1.40", "192.168.1.50", "172.17.0.1"]);
    }

    #[test]
    fn virtual_adapters_are_appended_after_real_ones() {
        let ips = relevant_ips(vec![
            iface("veth1234", v4([10, 0, 1, 2])),
            iface("br-abc", v4([10, 0, 2, 2])),
            iface("wlan0", v4([192, 168, 1, 50])),
        ]);
        assert_eq!(ips, vec!["192.168.1.50", "10.0.1.2", "10.0.2.2"]);
    }

    #[test]
    fn loopback_link_local_and_v6_are_dropped() {
        let ips = relevant_ips(vec![
            iface("lo", v4([127, 0, 0, 1])),
            iface("wlan0", v4([169, 254, 10, 10])),
            iface("wlan0", v4([0, 0, 0, 0])),
            iface("wlan0", IpAddr::V6("fe80::1".parse().unwrap())),
            iface("wlan0", v4([192, 168, 1, 50])),
        ]);
        assert_eq!(ips, vec!["192.168.1.50"]);
    }

    #[test]
    fn returns_parsable_addresses() {
        // No network assumptions: just check shape and validity.
        let ips = local_ips();
        for ip in &ips {
            assert!(
                ip.parse::<IpAddr>().is_ok(),
                "local_ips() returned a non-IP string: {ip:?}"
            );
        }
    }
}
