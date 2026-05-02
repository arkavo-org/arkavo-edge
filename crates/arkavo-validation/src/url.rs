use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};

/// Filter for outbound URLs to prevent SSRF attacks.
pub struct EgressFilter {
    blocked_ips: HashSet<IpAddr>,
    blocked_ranges: Vec<ipnet::IpNet>,
    allowlist: HashSet<String>,
    /// ARP §12.1: TTL for dynamically-added deny entries (seconds).
    pub arp_dynamic_deny_ttl_sec: Option<u64>,
    /// ARP §12.1: Consecutive connection failures before auto-deny.
    pub arp_connection_failure_threshold: Option<u32>,
    /// ARP §12.1: Auto-deny on TLS errors.
    pub arp_deny_on_tls_error: bool,
    /// ARP §12.1: Auto-deny on suspicious (injection) responses.
    pub arp_deny_on_suspicious_response: bool,
}

impl Default for EgressFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl EgressFilter {
    pub fn new() -> Self {
        let mut filter = Self {
            blocked_ips: HashSet::new(),
            blocked_ranges: Vec::new(),
            allowlist: HashSet::new(),
            arp_dynamic_deny_ttl_sec: None,
            arp_connection_failure_threshold: None,
            arp_deny_on_tls_error: false,
            arp_deny_on_suspicious_response: false,
        };
        filter.add_default_blocks();
        filter
    }

    /// Create an egress filter with ARP network egress config (§12.1) merged
    /// on top of the default static blocks. ARP can add dynamic deny triggers
    /// and TTL, but cannot remove the hardcoded SSRF protection ranges.
    pub fn with_arp(arp: &arkavo_arp::layers::Egress) -> Self {
        let mut filter = Self::new();
        filter.arp_dynamic_deny_ttl_sec = arp.dynamic_deny_ttl_sec;
        if let Some(deny) = &arp.dynamic_deny {
            filter.arp_connection_failure_threshold = deny.on_connection_failure_count;
            filter.arp_deny_on_tls_error = deny.on_tls_error.unwrap_or(false);
            filter.arp_deny_on_suspicious_response = deny.on_suspicious_response.unwrap_or(false);
        }
        filter
    }

    fn add_default_blocks(&mut self) {
        self.blocked_ips
            .insert(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)));

        self.blocked_ranges.push("10.0.0.0/8".parse().unwrap());
        self.blocked_ranges.push("172.16.0.0/12".parse().unwrap());
        self.blocked_ranges.push("192.168.0.0/16".parse().unwrap());
        self.blocked_ranges.push("127.0.0.0/8".parse().unwrap());
        self.blocked_ranges.push("169.254.0.0/16".parse().unwrap());
        self.blocked_ranges.push("fc00::/7".parse().unwrap());
        self.blocked_ranges.push("fe80::/10".parse().unwrap());
        self.blocked_ranges.push("::1/128".parse().unwrap());
        // IPv4-mapped IPv6 addresses (e.g. ::ffff:169.254.169.254)
        self.blocked_ranges
            .push("::ffff:10.0.0.0/104".parse().unwrap());
        self.blocked_ranges
            .push("::ffff:172.16.0.0/108".parse().unwrap());
        self.blocked_ranges
            .push("::ffff:192.168.0.0/112".parse().unwrap());
        self.blocked_ranges
            .push("::ffff:127.0.0.0/104".parse().unwrap());
        self.blocked_ranges
            .push("::ffff:169.254.0.0/112".parse().unwrap());
    }

    pub fn allow(&mut self, url: impl Into<String>) {
        self.allowlist.insert(url.into());
    }

    pub fn is_allowed(&self, url: &str) -> Result<(), EgressError> {
        if self.allowlist.contains(url) {
            return Ok(());
        }

        let host = extract_host_from_url(url).ok_or(EgressError::InvalidUrl)?;

        if let Ok(ip) = host.parse::<IpAddr>() {
            if self.is_ip_blocked(ip) {
                return Err(EgressError::BlockedIp(ip));
            }
            return Ok(());
        }

        let host_lower = host.to_lowercase();
        if host_lower == "localhost" || host_lower.starts_with("localhost.") {
            return Err(EgressError::BlockedIp(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        }

        Ok(())
    }

    fn is_ip_blocked(&self, ip: IpAddr) -> bool {
        if self.blocked_ips.contains(&ip) {
            return true;
        }
        self.blocked_ranges.iter().any(|range| range.contains(&ip))
    }

    pub fn validate_resolved_ip(&self, ip: IpAddr) -> Result<(), EgressError> {
        if self.is_ip_blocked(ip) {
            return Err(EgressError::BlockedIp(ip));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum EgressError {
    InvalidUrl,
    BlockedIp(IpAddr),
    BlockedDomain(String),
}

impl std::fmt::Display for EgressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EgressError::InvalidUrl => write!(f, "Invalid URL"),
            EgressError::BlockedIp(ip) => {
                write!(f, "SSRF attempt blocked: {ip} is a private/metadata IP")
            }
            EgressError::BlockedDomain(domain) => {
                write!(f, "SSRF attempt blocked: {domain} is in blocked list")
            }
        }
    }
}

impl std::error::Error for EgressError {}

/// Extract host from a URL string.
pub fn extract_host_from_url(url: &str) -> Option<String> {
    let without_protocol = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let host_part = without_protocol.split('/').next()?;
    // Strip userinfo@ (RFC 3986 §3.2.1) to prevent SSRF bypass
    let host_part = host_part.rsplit('@').next().unwrap_or(host_part);
    // Handle IPv6 bracket notation before stripping port
    let host = if host_part.starts_with('[') {
        host_part
            .split(']')
            .next()
            .map(|h| h.trim_start_matches('['))
            .unwrap_or(host_part)
    } else {
        host_part.split(':').next().unwrap_or(host_part)
    };
    Some(host.to_string())
}

/// Validator for HTTP Host headers to prevent DNS rebinding.
pub struct HostValidator {
    allowed_hosts: HashSet<String>,
}

impl Default for HostValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl HostValidator {
    pub fn new() -> Self {
        let mut validator = Self {
            allowed_hosts: HashSet::new(),
        };
        validator.allow("localhost");
        validator.allow("127.0.0.1");
        validator.allow("::1");
        validator
    }

    pub fn allow(&mut self, host: impl Into<String>) {
        self.allowed_hosts.insert(host.into().to_lowercase());
    }

    pub fn validate(&self, host: &str) -> Result<(), HostValidationError> {
        let host_lower = host.to_lowercase();
        let host_without_port = if host_lower.starts_with('[') {
            host_lower
                .trim_start_matches('[')
                .split("]:")
                .next()
                .unwrap_or(&host_lower)
                .trim_end_matches(']')
        } else {
            host_lower.split(':').next().unwrap_or(&host_lower)
        };

        if self.allowed_hosts.contains(host_without_port) {
            Ok(())
        } else {
            Err(HostValidationError::HostNotAllowed(host.to_string()))
        }
    }

    pub fn is_localhost(&self, host: &str) -> bool {
        let host_lower = host.to_lowercase();
        let host_normalized = if host_lower.starts_with('[') {
            host_lower
                .trim_start_matches('[')
                .split("]:")
                .next()
                .unwrap_or(&host_lower)
                .trim_end_matches(']')
        } else {
            host_lower.split(':').next().unwrap_or(&host_lower)
        };
        matches!(host_normalized, "localhost" | "127.0.0.1" | "::1")
    }
}

#[derive(Debug, Clone)]
pub enum HostValidationError {
    HostNotAllowed(String),
}

impl std::fmt::Display for HostValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostValidationError::HostNotAllowed(host) => {
                write!(
                    f,
                    "Host '{host}' not allowed (potential DNS rebinding attack)"
                )
            }
        }
    }
}

impl std::error::Error for HostValidationError {}

/// Check if a host string refers to the loopback interface.
/// Handles IPv6 bracket notation (e.g. `[::1]`).
pub fn is_loopback_host(host: &str) -> bool {
    let host_lower = host.to_lowercase();
    if host_lower == "localhost" {
        return true;
    }
    // Strip IPv6 brackets if present
    let bare = host_lower
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(&host_lower);
    match bare.parse::<IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for URL validation and egress filtering.
    //!
    //! ## Spec Coverage
    //! - [specs/arkavo-edge/network-security.spec.yaml](NET-007): Block cloud metadata and internal network access
    //! - [specs/arkavo-edge/network-security.spec.yaml](HIGH-004): Host header validation (DNS rebinding)
    //!
    //! ## Vulnerability IDs
    //! - CRI-003: Egress filtering (SSRF prevention)

    use super::*;
    use arkavo_test_macros::spec;

    /// Test: Egress filter blocks access to private IP ranges and cloud metadata
    /// Spec: NET-007 - Block cloud metadata and internal network access
    /// Vulnerability: CRI-003 - SSRF Prevention
    #[spec("VAL-005")]
    #[test]
    fn test_egress_filter_blocks_private_ips() {
        let filter = EgressFilter::new();
        assert!(filter.is_allowed("http://10.0.0.1/api").is_err());
        assert!(filter.is_allowed("http://192.168.1.1/api").is_err());
        assert!(filter.is_allowed("http://172.16.0.1/api").is_err());
        assert!(
            filter
                .is_allowed("http://169.254.169.254/latest/meta-data")
                .is_err()
        );
        assert!(filter.is_allowed("http://localhost/api").is_err());
    }

    /// Test: Egress filter allows legitimate public URLs
    /// Spec: NET-007 - Public IPs should be accessible
    #[spec("VAL-005")]
    #[test]
    fn test_egress_filter_allows_public_urls() {
        let filter = EgressFilter::new();
        assert!(filter.is_allowed("https://api.example.com/v1").is_ok());
        assert!(filter.is_allowed("https://github.com/repo").is_ok());
    }

    /// Test: Allowlist can bypass egress filter for specific URLs
    /// Spec: NET-007 - Explicit allowlist can override for known-safe internal services
    #[spec("VAL-005")]
    #[test]
    fn test_egress_filter_allowlist() {
        let mut filter = EgressFilter::new();
        filter.allow("http://localhost/api");
        assert!(filter.is_allowed("http://localhost/api").is_ok());
    }

    /// Test: Host validator prevents DNS rebinding attacks
    /// Spec: NET-006 - Host header validation (anti-rebinding)
    /// Spec: HIGH-004 - Host header validation
    #[spec("VAL-006")]
    #[test]
    fn test_host_validator() {
        let validator = HostValidator::new();
        assert!(validator.validate("localhost").is_ok());
        assert!(validator.validate("localhost:8080").is_ok());
        assert!(validator.validate("127.0.0.1").is_ok());
        assert!(validator.validate("[::1]:8080").is_ok());
        assert!(validator.validate("evil.com").is_err());
    }

    /// Test: Loopback host detection for local binding validation
    /// Spec: NET-002 - Localhost-only binding by default
    #[spec("VAL-006")]
    #[test]
    fn test_is_loopback_host() {
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("::1"));
        assert!(!is_loopback_host("192.168.1.1"));
        assert!(!is_loopback_host("example.com"));
    }

    #[spec("VAL-006")]
    #[test]
    fn test_extract_host_from_url() {
        assert_eq!(
            extract_host_from_url("https://example.com:8080/path"),
            Some("example.com".to_string())
        );
        assert_eq!(extract_host_from_url("not-a-url"), None);
    }

    #[spec("VAL-005", "VAL-006")]
    #[test]
    fn test_userinfo_bypass_blocked() {
        assert_eq!(
            extract_host_from_url("http://evil@169.254.169.254/meta"),
            Some("169.254.169.254".to_string())
        );
    }

    #[spec("VAL-006")]
    #[test]
    fn test_ipv6_url_extraction() {
        assert_eq!(
            extract_host_from_url("http://[::1]:8080/path"),
            Some("::1".to_string())
        );
        assert_eq!(
            extract_host_from_url("http://[::ffff:169.254.169.254]/"),
            Some("::ffff:169.254.169.254".to_string())
        );
    }

    #[spec("VAL-005")]
    #[test]
    fn test_egress_filter_from_arp_preserves_defaults() {
        let arp_egress = arkavo_arp::layers::Egress {
            dynamic_deny: None,
            deny_takes_precedence: Some(true),
            dynamic_deny_ttl_sec: Some(3600),
        };
        let filter = EgressFilter::with_arp(&arp_egress);
        assert!(filter.is_allowed("http://10.0.0.1/api").is_err());
        assert!(
            filter
                .is_allowed("http://169.254.169.254/latest/meta-data")
                .is_err()
        );
        assert!(filter.is_allowed("https://api.example.com/v1").is_ok());
        assert_eq!(filter.arp_dynamic_deny_ttl_sec, Some(3600));
    }

    #[test]
    fn test_egress_filter_from_arp_dynamic_deny_config() {
        let arp_egress = arkavo_arp::layers::Egress {
            dynamic_deny: Some(arkavo_arp::layers::DynamicDeny {
                on_connection_failure_count: Some(3),
                on_tls_error: Some(true),
                on_suspicious_response: Some(true),
            }),
            deny_takes_precedence: Some(true),
            dynamic_deny_ttl_sec: Some(7200),
        };
        let filter = EgressFilter::with_arp(&arp_egress);
        assert_eq!(filter.arp_connection_failure_threshold, Some(3));
        assert!(filter.arp_deny_on_tls_error);
        assert!(filter.arp_deny_on_suspicious_response);
    }

    #[test]
    fn test_ipv6_mapped_v4_blocked() {
        let filter = EgressFilter::new();
        assert!(
            filter
                .is_allowed("http://[::ffff:169.254.169.254]/")
                .is_err()
        );
        assert!(filter.is_allowed("http://[::ffff:10.0.0.1]/").is_err());
        assert!(filter.is_allowed("http://[::ffff:192.168.1.1]/").is_err());
    }
}
