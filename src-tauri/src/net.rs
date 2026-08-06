//! Local network helpers.

use std::net::IpAddr;

/// All usable local IPv4 addresses (non-loopback, non-link-local), sorted,
/// as strings. Returns an empty vec if the interface list can't be read
/// (never panics).
pub fn local_ips() -> Vec<String> {
    let Ok(interfaces) = local_ip_address::list_afinet_netifas() else {
        return Vec::new();
    };
    let mut ips: Vec<String> = interfaces
        .into_iter()
        .map(|(_, ip)| ip)
        .filter(|ip| match ip {
            // IPv4 only; drop loopback, link-local and unspecified.
            IpAddr::V4(v4) => !v4.is_loopback() && !v4.is_link_local() && !v4.is_unspecified(),
            IpAddr::V6(_) => false,
        })
        .map(|ip| ip.to_string())
        .collect();
    ips.sort();
    ips.dedup();
    ips
}

#[cfg(test)]
mod tests {
    use super::*;

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
