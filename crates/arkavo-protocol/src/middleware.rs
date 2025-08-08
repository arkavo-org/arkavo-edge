use dashmap::DashMap;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use tracing::{debug, warn};

/// Connection tracking for associating connection IDs with IP addresses
pub struct ConnectionTracker {
    connections: Arc<DashMap<u64, ClientInfo>>,
    trusted_proxies: Vec<IpAddr>,
    enable_x_forwarded_for: bool,
}

#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub ip: IpAddr,
    pub connection_id: u64,
    pub headers: Option<HeaderMap>,
}

/// Simple header map for storing HTTP headers
#[derive(Debug, Clone, Default)]
pub struct HeaderMap {
    headers: Vec<(String, String)>,
}

impl HeaderMap {
    pub fn new() -> Self {
        Self {
            headers: Vec::new(),
        }
    }

    pub fn insert(&mut self, key: String, value: String) {
        self.headers.push((key.to_lowercase(), value));
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        let key_lower = key.to_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k == &key_lower)
            .map(|(_, v)| v.as_str())
    }
}

impl ConnectionTracker {
    pub fn new(trusted_proxies: Vec<String>, enable_x_forwarded_for: bool) -> Self {
        let proxy_ips: Vec<IpAddr> = trusted_proxies
            .iter()
            .filter_map(|s| IpAddr::from_str(s).ok())
            .collect();

        if proxy_ips.len() != trusted_proxies.len() {
            warn!("Some trusted proxy addresses could not be parsed");
        }

        Self {
            connections: Arc::new(DashMap::new()),
            trusted_proxies: proxy_ips,
            enable_x_forwarded_for,
        }
    }

    /// Register a new connection
    pub fn register_connection(
        &self,
        connection_id: u64,
        socket_addr: SocketAddr,
        headers: Option<HeaderMap>,
    ) {
        let ip = self.extract_client_ip(socket_addr, headers.as_ref());

        let info = ClientInfo {
            ip,
            connection_id,
            headers,
        };

        debug!(connection_id = connection_id, ip = ?ip, "Registered new connection");
        self.connections.insert(connection_id, info);
    }

    /// Get client IP for a connection
    pub fn get_client_ip(&self, connection_id: u64) -> Option<IpAddr> {
        self.connections.get(&connection_id).map(|info| info.ip)
    }

    /// Remove a connection
    pub fn remove_connection(&self, connection_id: u64) {
        if let Some((_, info)) = self.connections.remove(&connection_id) {
            debug!(connection_id = connection_id, ip = ?info.ip, "Removed connection");
        }
    }

    /// Extract the real client IP considering proxy headers
    fn extract_client_ip(&self, socket_addr: SocketAddr, headers: Option<&HeaderMap>) -> IpAddr {
        let socket_ip = socket_addr.ip();

        // If X-Forwarded-For is disabled or no headers, use socket IP
        if !self.enable_x_forwarded_for || headers.is_none() {
            return socket_ip;
        }

        // Check if the connection is from a trusted proxy
        if !self.is_trusted_proxy(&socket_ip) {
            return socket_ip;
        }

        let headers = headers.unwrap();

        // Try X-Forwarded-For header first
        if let Some(forwarded_for) = headers.get("x-forwarded-for") {
            // X-Forwarded-For can contain multiple IPs: "client, proxy1, proxy2"
            // We want the first (leftmost) IP which is the original client
            if let Some(first_ip) = forwarded_for.split(',').next() {
                if let Ok(ip) = IpAddr::from_str(first_ip.trim()) {
                    debug!(forwarded_ip = ?ip, socket_ip = ?socket_ip, "Using X-Forwarded-For IP");
                    return ip;
                }
            }
        }

        // Try X-Real-IP header (used by some proxies like nginx)
        if let Some(real_ip) = headers.get("x-real-ip") {
            if let Ok(ip) = IpAddr::from_str(real_ip.trim()) {
                debug!(real_ip = ?ip, socket_ip = ?socket_ip, "Using X-Real-IP");
                return ip;
            }
        }

        // Try Forwarded header (RFC 7239)
        if let Some(forwarded) = headers.get("forwarded") {
            // Parse "for=192.0.2.60;proto=http;by=198.51.100.17"
            for param in forwarded.split(';') {
                let param = param.trim();
                if param.starts_with("for=") {
                    let for_value = &param[4..];
                    // Remove quotes and brackets if present
                    let for_value = for_value
                        .trim_matches('"')
                        .trim_matches('[')
                        .trim_matches(']');
                    if let Ok(ip) = IpAddr::from_str(for_value) {
                        debug!(forwarded_ip = ?ip, socket_ip = ?socket_ip, "Using Forwarded header IP");
                        return ip;
                    }
                }
            }
        }

        // Fall back to socket IP if no valid forwarded IP found
        debug!(socket_ip = ?socket_ip, "No valid forwarded IP found, using socket IP");
        socket_ip
    }

    /// Check if an IP is in the trusted proxy list
    fn is_trusted_proxy(&self, ip: &IpAddr) -> bool {
        self.trusted_proxies.contains(ip)
    }

    /// Get the number of active connections
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Clear all connections (useful for testing)
    #[cfg(test)]
    pub fn clear(&self) {
        self.connections.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_connection_tracker_basic() {
        let tracker = ConnectionTracker::new(vec![], false);
        let addr: SocketAddr = "192.168.1.100:8080".parse().unwrap();

        tracker.register_connection(1, addr, None);

        let ip = tracker.get_client_ip(1);
        assert_eq!(ip, Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100))));

        tracker.remove_connection(1);
        assert_eq!(tracker.get_client_ip(1), None);
    }

    #[test]
    fn test_x_forwarded_for_trusted() {
        let tracker = ConnectionTracker::new(vec!["10.0.0.1".to_string()], true);

        let proxy_addr: SocketAddr = "10.0.0.1:8080".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for".to_string(), "203.0.113.195".to_string());

        tracker.register_connection(1, proxy_addr, Some(headers));

        let ip = tracker.get_client_ip(1);
        assert_eq!(ip, Some(IpAddr::from_str("203.0.113.195").unwrap()));
    }

    #[test]
    fn test_x_forwarded_for_untrusted() {
        let tracker = ConnectionTracker::new(vec!["10.0.0.1".to_string()], true);

        // Connection from untrusted IP
        let untrusted_addr: SocketAddr = "192.168.1.100:8080".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for".to_string(), "203.0.113.195".to_string());

        tracker.register_connection(1, untrusted_addr, Some(headers));

        // Should use socket IP, not forwarded IP
        let ip = tracker.get_client_ip(1);
        assert_eq!(ip, Some(IpAddr::from_str("192.168.1.100").unwrap()));
    }

    #[test]
    fn test_x_forwarded_for_multiple_ips() {
        let tracker = ConnectionTracker::new(vec!["10.0.0.1".to_string()], true);

        let proxy_addr: SocketAddr = "10.0.0.1:8080".parse().unwrap();
        let mut headers = HeaderMap::new();
        // Multiple IPs in X-Forwarded-For
        headers.insert(
            "x-forwarded-for".to_string(),
            "203.0.113.195, 198.51.100.178, 10.0.0.1".to_string(),
        );

        tracker.register_connection(1, proxy_addr, Some(headers));

        // Should use the first (leftmost) IP
        let ip = tracker.get_client_ip(1);
        assert_eq!(ip, Some(IpAddr::from_str("203.0.113.195").unwrap()));
    }

    #[test]
    fn test_x_real_ip_fallback() {
        let tracker = ConnectionTracker::new(vec!["10.0.0.1".to_string()], true);

        let proxy_addr: SocketAddr = "10.0.0.1:8080".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip".to_string(), "203.0.113.195".to_string());

        tracker.register_connection(1, proxy_addr, Some(headers));

        let ip = tracker.get_client_ip(1);
        assert_eq!(ip, Some(IpAddr::from_str("203.0.113.195").unwrap()));
    }

    #[test]
    fn test_forwarded_header() {
        let tracker = ConnectionTracker::new(vec!["10.0.0.1".to_string()], true);

        let proxy_addr: SocketAddr = "10.0.0.1:8080".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "forwarded".to_string(),
            "for=192.0.2.60;proto=http;by=10.0.0.1".to_string(),
        );

        tracker.register_connection(1, proxy_addr, Some(headers));

        let ip = tracker.get_client_ip(1);
        assert_eq!(ip, Some(IpAddr::from_str("192.0.2.60").unwrap()));
    }
}
