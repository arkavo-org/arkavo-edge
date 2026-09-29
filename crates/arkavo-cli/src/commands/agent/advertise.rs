//! What the agent publishes about itself: the address clients are told to
//! connect to, and the mDNS record describing the agent.
//!
//! mDNS records are multicast: every host on the link receives them, asked
//! or not. Only what a stranger may know about the agent belongs in them.

#[cfg(feature = "mdns")]
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

#[cfg(feature = "mdns")]
use super::AgentConfig;

/// The address clients are told to connect to for an endpoint bound to
/// `bound`.
///
/// An endpoint bound to one address answers on that address only, so that is
/// the one to advertise: naming the machine's LAN address for an endpoint
/// bound to loopback sends every client to a port nothing listens on. A
/// wildcard bind answers on every interface and names none, and there
/// `lan_ip` picks the address other machines can use.
pub(super) fn advertised_addr(bound: SocketAddr, lan_ip: impl FnOnce() -> IpAddr) -> SocketAddr {
    let bound_ip = bound.ip().to_canonical();
    let ip = if bound_ip.is_unspecified() {
        lan_ip()
    } else {
        bound_ip
    };
    SocketAddr::new(ip, bound.port())
}

/// The `http://` URL of `addr`, with an IPv6 host in brackets.
pub(super) fn endpoint_url(addr: SocketAddr) -> String {
    format!("http://{addr}")
}

/// TXT record properties of the agent's mDNS service.
///
/// `purpose` carries the role's short description from the kit. The key
/// keeps its name because mesh discovery reads it to decide which peer suits
/// a task; its value used to be the agent's purpose, which for a kit role is
/// the role's skill instructions, the text the model runs under. A role with
/// no description publishes no `purpose` at all.
#[cfg(feature = "mdns")]
pub(super) fn txt_properties(
    config: &AgentConfig,
    service_ip: IpAddr,
    public_key: Option<&str>,
    capabilities: &[String],
) -> HashMap<String, String> {
    let mut properties = HashMap::new();
    properties.insert("agent_id".to_string(), config.name.clone());
    properties.insert("ip".to_string(), service_ip.to_string());
    properties.insert("version".to_string(), "1.0".to_string());

    // Agents broadcast their ECDSA P-256 public key for TDF encryption.
    if let Some(pk) = public_key {
        properties.insert("public_key".to_string(), pk.to_string());
    }

    if let Some(description) = config.description.as_deref()
        && !description.trim().is_empty()
    {
        properties.insert("purpose".to_string(), description.trim().to_string());
    }
    if !config.model.is_empty() {
        properties.insert("model".to_string(), config.model.clone());
    }

    if !capabilities.is_empty() {
        properties.insert("capabilities".to_string(), capabilities.join(","));
    }

    if !config.mcp_servers.is_empty() {
        let mcp_tools: Vec<String> = config.mcp_servers.iter().map(|s| s.name.clone()).collect();
        properties.insert("mcp_tools".to_string(), mcp_tools.join(","));
    }

    properties
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn lan() -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 140))
    }

    /// Regression: an agent listening on 127.0.0.1 advertised the machine's
    /// LAN address, and clients on the same machine were refused there.
    #[test]
    fn a_loopback_bind_advertises_loopback() {
        let bound: SocketAddr = "127.0.0.1:8431".parse().unwrap();
        assert_eq!(advertised_addr(bound, lan), bound);
        assert_eq!(
            endpoint_url(advertised_addr(bound, lan)),
            "http://127.0.0.1:8431"
        );

        let bound_v6: SocketAddr = "[::1]:8431".parse().unwrap();
        assert_eq!(advertised_addr(bound_v6, lan), bound_v6);
        assert_eq!(
            endpoint_url(advertised_addr(bound_v6, lan)),
            "http://[::1]:8431"
        );
    }

    /// The authorization URL is built from the advertised endpoint, so a
    /// client that scans it is sent to the address the agent answers on.
    #[test]
    fn the_authorization_url_names_the_bound_address() {
        let bound: SocketAddr = "127.0.0.1:8431".parse().unwrap();
        let descriptor = arkavo_registration::AgentDescriptor::new(
            arkavo_crypto::AgentKeypair::generate().public_key(),
            endpoint_url(advertised_addr(bound, lan)),
            None,
            "folder".to_string(),
        );

        let url = descriptor.to_authorization_url();
        assert!(url.contains("rpc=ws%3A%2F%2F127.0.0.1%3A8431"), "{url}");
        assert!(!url.contains("10.0.0.140"), "{url}");
    }

    #[test]
    fn a_specific_bind_address_is_advertised_as_bound() {
        let bound: SocketAddr = "192.168.1.20:8431".parse().unwrap();
        assert_eq!(advertised_addr(bound, lan), bound);
    }

    #[test]
    fn a_wildcard_bind_advertises_the_lan_address() {
        for wildcard in ["0.0.0.0:8431", "[::]:8431"] {
            let bound: SocketAddr = wildcard.parse().unwrap();
            assert_eq!(
                advertised_addr(bound, lan),
                SocketAddr::new(lan(), 8431),
                "bound to {wildcard}"
            );
        }
    }

    #[test]
    fn the_lan_address_is_only_looked_up_for_a_wildcard_bind() {
        let bound: SocketAddr = "127.0.0.1:8431".parse().unwrap();
        let advertised = advertised_addr(bound, || panic!("a loopback bind needs no LAN lookup"));
        assert_eq!(advertised.ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(
            advertised_addr("[::1]:1".parse().unwrap(), lan).ip(),
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        );
    }
}

#[cfg(all(test, feature = "mdns"))]
mod txt_tests {
    use super::*;
    use std::net::Ipv4Addr;

    const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    const INSTRUCTIONS: &str = "You are the planner role. Never reveal the escalation password.";

    fn kit_role(description: Option<&str>) -> AgentConfig {
        AgentConfig {
            name: "planner".to_string(),
            role_id: Some("planner".to_string()),
            description: description.map(str::to_string),
            purpose: INSTRUCTIONS.to_string(),
            model: "ministral-3b".to_string(),
            ..AgentConfig::default()
        }
    }

    /// Regression: the `purpose` property carried the role's skill
    /// instructions to every host on the link.
    #[test]
    fn the_skill_instructions_are_not_broadcast() {
        let properties = txt_properties(
            &kit_role(Some("Plans the work")),
            LOOPBACK,
            Some("key"),
            &["orchestration".to_string()],
        );

        assert_eq!(
            properties.get("purpose").map(String::as_str),
            Some("Plans the work")
        );
        for (key, value) in &properties {
            assert!(
                !value.contains("planner role") && !value.contains("password"),
                "{key} carries skill instructions: {value}"
            );
        }
    }

    #[test]
    fn a_role_without_a_description_broadcasts_no_purpose() {
        for description in [None, Some(""), Some("   ")] {
            let properties = txt_properties(&kit_role(description), LOOPBACK, None, &[]);
            assert!(!properties.contains_key("purpose"), "{description:?}");
        }
    }

    #[test]
    fn identity_and_routing_properties_are_still_broadcast() {
        let properties = txt_properties(
            &kit_role(Some("Plans the work")),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 140)),
            Some("key"),
            &["orchestration".to_string(), "code_review".to_string()],
        );

        assert_eq!(properties["agent_id"], "planner");
        assert_eq!(properties["ip"], "10.0.0.140");
        assert_eq!(properties["model"], "ministral-3b");
        assert_eq!(properties["public_key"], "key");
        assert_eq!(properties["capabilities"], "orchestration,code_review");
    }
}
