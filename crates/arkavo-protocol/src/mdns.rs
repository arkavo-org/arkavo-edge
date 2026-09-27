//! mDNS service discovery for A2A agents
//!
//! This module provides mDNS service registration and discovery functionality
//! for A2A agents to find each other on the local network.

#[cfg(feature = "mdns")]
use crate::network::get_service_ip;
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

/// Information about an mDNS service for A2A agent registration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MdnsServiceInfo {
    /// Unique identifier for the agent
    pub agent_id: String,
    /// Human-readable name for the agent
    pub name: String,
    /// Port the agent is listening on
    pub port: u16,
    /// Purpose or description of the agent
    pub purpose: String,
    /// LLM model the agent is using
    pub model: String,
    /// Optional IP address override (if not provided, network IP will be detected)
    pub ip: Option<Ipv4Addr>,
}

impl MdnsServiceInfo {
    /// Create a new MdnsServiceInfo
    pub fn new(
        agent_id: impl Into<String>,
        name: impl Into<String>,
        port: u16,
        purpose: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            name: name.into(),
            port,
            purpose: purpose.into(),
            model: model.into(),
            ip: None,
        }
    }

    /// Set the IP address for the service
    pub fn with_ip(mut self, ip: Ipv4Addr) -> Self {
        self.ip = Some(ip);
        self
    }

    /// Get the IP address to use for service registration
    #[cfg(feature = "mdns")]
    pub fn get_service_ip(&self) -> Ipv4Addr {
        self.ip.unwrap_or_else(get_service_ip)
    }

    #[cfg(not(feature = "mdns"))]
    pub fn get_service_ip(&self) -> Ipv4Addr {
        self.ip.unwrap_or_else(|| Ipv4Addr::new(127, 0, 0, 1))
    }
}

/// Errors that can occur during mDNS operations
#[derive(Debug, thiserror::Error)]
pub enum MdnsError {
    #[error("mDNS feature not compiled in")]
    FeatureNotEnabled,
    #[error("mDNS service registration failed: {0}")]
    RegistrationFailed(String),
    #[error("mDNS discovery failed: {0}")]
    DiscoveryFailed(String),
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("PROTO-004")]
    #[test]
    fn test_mdns_service_info_creation() {
        let info = MdnsServiceInfo::new(
            "test-agent",
            "Test Agent",
            8342,
            "Testing purposes",
            "gpt-4",
        );

        assert_eq!(info.agent_id, "test-agent");
        assert_eq!(info.name, "Test Agent");
        assert_eq!(info.port, 8342);
        assert_eq!(info.purpose, "Testing purposes");
        assert_eq!(info.model, "gpt-4");
        assert!(info.ip.is_none());
    }

    #[spec("PROTO-004")]
    #[test]
    fn test_mdns_service_info_with_ip() {
        let ip = Ipv4Addr::new(192, 168, 1, 100);
        let info = MdnsServiceInfo::new("test", "Test", 8342, "Test", "model").with_ip(ip);

        assert_eq!(info.ip, Some(ip));
        assert_eq!(info.get_service_ip(), ip);
    }

    #[spec("PROTO-004")]
    #[test]
    fn test_get_service_ip_fallback() {
        let info = MdnsServiceInfo::new("test", "Test", 8342, "Test", "model");
        let ip = info.get_service_ip();

        // Should return either the detected network IP or localhost
        assert!(!ip.is_unspecified()); // Not 0.0.0.0
    }
}
