#[cfg(feature = "mdns")]
use std::net::IpAddr;
use std::net::Ipv4Addr;
#[cfg(feature = "mdns")]
use tracing::debug;
use tracing::warn;

/// Errors that can occur during network interface detection
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("Failed to enumerate network interfaces: {0}")]
    InterfaceEnumeration(String),
    #[error("No suitable network interface found")]
    NoSuitableInterface,
}

/// Get the best network IP address for mDNS service registration
///
/// This function finds a suitable IPv4 address that can be used for
/// advertising services across the local network mesh. It prioritizes:
/// 1. Private network ranges (192.168.x.x, 10.x.x.x, 172.16-31.x.x)
/// 2. Non-loopback addresses
/// 3. Non-zero addresses (not 0.0.0.0)
#[cfg(feature = "mdns")]
pub fn get_network_ip() -> Result<Ipv4Addr, NetworkError> {
    use get_if_addrs::get_if_addrs;

    let interfaces =
        get_if_addrs().map_err(|e| NetworkError::InterfaceEnumeration(e.to_string()))?;

    debug!("Found {} network interfaces", interfaces.len());

    let mut candidates = Vec::new();

    for interface in interfaces {
        if let IpAddr::V4(ipv4) = interface.ip() {
            debug!("Evaluating interface {}: {}", interface.name, ipv4);

            // Skip loopback addresses
            if ipv4.is_loopback() {
                debug!("Skipping loopback address: {}", ipv4);
                continue;
            }

            // Skip 0.0.0.0
            if ipv4.octets() == [0, 0, 0, 0] {
                debug!("Skipping zero address: {}", ipv4);
                continue;
            }

            // Skip link-local addresses (169.254.x.x)
            if ipv4.octets()[0] == 169 && ipv4.octets()[1] == 254 {
                debug!("Skipping link-local address: {}", ipv4);
                continue;
            }

            candidates.push((ipv4, interface.name.clone()));
        }
    }

    if candidates.is_empty() {
        warn!("No suitable IPv4 interfaces found");
        return Err(NetworkError::NoSuitableInterface);
    }

    // Sort by preference: private networks first, then others
    candidates.sort_by_key(|(ip, _)| {
        let octets = ip.octets();
        match octets {
            // 192.168.x.x - highest priority
            [192, 168, _, _] => 0,
            // 10.x.x.x - second priority
            [10, _, _, _] => 1,
            // 172.16-31.x.x - third priority
            [172, second, _, _] if (16..=31).contains(&second) => 2,
            // Any other address - lowest priority
            _ => 3,
        }
    });

    let (best_ip, interface_name) = &candidates[0];
    debug!("Selected network interface {}: {}", interface_name, best_ip);

    Ok(*best_ip)
}

/// Get a fallback IP address when network detection fails
///
/// Returns 127.0.0.1 as a last resort for local testing
pub fn get_fallback_ip() -> Ipv4Addr {
    warn!("Using fallback IP address 127.0.0.1");
    Ipv4Addr::LOCALHOST
}

/// Get the best available IP address for service registration
///
/// This function attempts to get a network IP, falling back to localhost
/// if network detection fails
#[cfg(feature = "mdns")]
pub fn get_service_ip() -> Ipv4Addr {
    match get_network_ip() {
        Ok(ip) => ip,
        Err(e) => {
            warn!("Failed to detect network IP: {}, using fallback", e);
            get_fallback_ip()
        }
    }
}

#[cfg(not(feature = "mdns"))]
pub fn get_service_ip() -> Ipv4Addr {
    get_fallback_ip()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_ip() {
        let ip = get_fallback_ip();
        assert_eq!(ip, Ipv4Addr::LOCALHOST);
    }

    #[test]
    fn test_service_ip_returns_valid_address() {
        let ip = get_service_ip();
        assert!(!ip.is_unspecified()); // Not 0.0.0.0
    }

    #[cfg(feature = "mdns")]
    #[test]
    fn test_network_detection_or_fallback() {
        // This test ensures we get either a valid network IP or fallback
        let ip = get_service_ip();
        let octets = ip.octets();

        // Should be either a valid private network or localhost
        let is_valid = matches!(
            octets,
            [127, 0, 0, 1] |           // localhost fallback
            [192, 168, _, _] |         // 192.168.x.x
            [10, _, _, _] |            // 10.x.x.x  
            [172, 16..=31, _, _] // 172.16-31.x.x
        ) || !ip.is_loopback() && !ip.is_unspecified();

        assert!(
            is_valid,
            "IP address {ip} should be valid for service registration"
        );
    }
}
