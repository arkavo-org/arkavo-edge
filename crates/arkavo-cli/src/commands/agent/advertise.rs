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
use super::listen::is_loopback;

/// The address clients are told to connect to for an endpoint bound to
/// `bound`.
///
/// An endpoint bound to one address answers on that address only, so that is
/// the one to advertise: naming the machine's LAN address for an endpoint
/// bound to loopback sends every client to a port nothing listens on. A
/// wildcard bind answers on every interface and names none, and there
/// `lan_ip` picks the address other machines can use.
pub(super) fn advertised_addr(bound: SocketAddr, lan_ip: impl FnOnce() -> IpAddr) -> SocketAddr {
    let ip = if is_wildcard(bound) {
        lan_ip()
    } else {
        bound.ip().to_canonical()
    };
    SocketAddr::new(ip, bound.port())
}

/// Whether `bound` is a wildcard bind, which answers on every interface.
fn is_wildcard(bound: SocketAddr) -> bool {
    bound.ip().to_canonical().is_unspecified()
}

/// The `http://` URL of `addr`, with an IPv6 host in brackets.
pub(super) fn endpoint_url(addr: SocketAddr) -> String {
    format!("http://{addr}")
}

/// Whether the agent announces itself over mDNS: when its kit asks for it
/// and other machines can reach the endpoint.
///
/// mDNS tells the machines on the network where the agent is. An endpoint
/// on loopback is not on the network, so there is nothing to announce.
pub(super) fn announced_over_mdns(mdns_enabled: bool, bound: SocketAddr) -> bool {
    mdns_enabled && !is_loopback(bound.ip())
}

/// The notice to show at startup when the kit asks for mDNS and the
/// endpoint is on loopback, `None` otherwise. Without it the agent would
/// be missing from discovery with nothing saying why.
pub(super) fn mdns_off_notice(mdns_enabled: bool, bound: SocketAddr) -> Option<String> {
    (mdns_enabled && is_loopback(bound.ip())).then(|| {
        format!(
            "Agent RPC endpoint {bound} is on this machine only, so the agent is not announced over mDNS.\n\
             Start the agent with --bind 0.0.0.0 to be found on the network."
        )
    })
}

/// The mDNS daemon that announces the agent and browses for its peers.
///
/// mdns-sd leaves the loopback interfaces out, and they stay out: nothing
/// on them is on the network. For a wildcard bind the daemon also keeps to
/// IPv4, because every peer reader builds `http://{addr}:{port}` from the
/// first address it resolves, which an IPv6 address does not survive. An
/// endpoint bound to one address is announced with it, whatever its family.
#[cfg(feature = "mdns")]
pub(super) fn mdns_daemon(bound: SocketAddr) -> mdns_sd::Result<mdns_sd::ServiceDaemon> {
    let daemon = mdns_sd::ServiceDaemon::new()?;
    if is_wildcard(bound) {
        daemon.disable_interface(mdns_sd::IfKind::IPv6)?;
    }
    Ok(daemon)
}

/// The mDNS record of an agent whose endpoint is bound to `bound`.
///
/// An endpoint bound to one address is announced with that address. A
/// wildcard bind answers on every interface, so the record takes its
/// addresses from the interfaces themselves: the daemon announces each one
/// on the interface that holds it, and follows addresses as they come and
/// go. A single address picked for the whole machine would be announced on
/// that address's network alone. The TXT `ip` is the address
/// [`advertised_addr`] gives, the one the authorization link names.
#[cfg(feature = "mdns")]
pub(super) fn service_info(
    config: &AgentConfig,
    bound: SocketAddr,
    lan_ip: impl FnOnce() -> IpAddr,
    public_key: Option<&str>,
    capabilities: &[String],
) -> mdns_sd::Result<mdns_sd::ServiceInfo> {
    let advertised = advertised_addr(bound, lan_ip).ip();
    let properties = txt_properties(config, advertised, public_key, capabilities);
    let wildcard = is_wildcard(bound);
    let addresses = if wildcard {
        Vec::new()
    } else {
        vec![advertised]
    };
    let info = mdns_sd::ServiceInfo::new(
        "_a2a._tcp.local.",
        &config.name,
        &format!("{}.local.", config.name),
        addresses.as_slice(),
        bound.port(),
        properties,
    )?;
    Ok(if wildcard {
        info.enable_addr_auto()
    } else {
        info
    })
}

/// TXT record properties of the agent's mDNS service.
///
/// `purpose` carries the role's short description from the kit. The key
/// keeps its name because mesh discovery reads it to decide which peer suits
/// a task; its value used to be the agent's purpose, which for a kit role is
/// the role's skill instructions, the text the model runs under. A role with
/// no description publishes no `purpose` at all.
#[cfg(feature = "mdns")]
fn txt_properties(
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

/// Withdraws the agent's mDNS record, waiting up to a second for the daemon
/// to send the goodbye. Dropping the daemon sends none, so peers kept the
/// agent and its gossip key, and refused the new key it came back with after
/// a restart.
#[cfg(feature = "mdns")]
pub(super) fn announce_departure(mdns: &mdns_sd::ServiceDaemon, fullname: &str) {
    if let Ok(done) = mdns.unregister(fullname) {
        let _ = done.recv_timeout(std::time::Duration::from_secs(1));
    }
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

    /// Regression (#729): an agent on loopback was announced on the
    /// loopback interface, where no other machine could see it.
    #[test]
    fn an_agent_on_loopback_is_not_announced() {
        for bound in ["127.0.0.1:8431", "[::1]:8431", "[::ffff:127.0.0.1]:8431"] {
            let bound: SocketAddr = bound.parse().unwrap();
            assert!(!announced_over_mdns(true, bound), "{bound}");
            let notice = mdns_off_notice(true, bound)
                .unwrap_or_else(|| panic!("{bound}: the missing announcement must be said"));
            assert!(notice.contains(&bound.to_string()), "{notice}");
            assert!(notice.contains("--bind 0.0.0.0"), "{notice}");
        }
    }

    #[test]
    fn an_agent_on_the_network_is_announced_without_a_notice() {
        for bound in ["0.0.0.0:8431", "[::]:8431", "10.0.0.140:8431"] {
            let bound: SocketAddr = bound.parse().unwrap();
            assert!(announced_over_mdns(true, bound), "{bound}");
            assert_eq!(mdns_off_notice(true, bound), None, "{bound}");
        }
    }

    #[test]
    fn an_agent_with_mdns_off_is_neither_announced_nor_noticed() {
        for bound in ["127.0.0.1:8431", "0.0.0.0:8431"] {
            let bound: SocketAddr = bound.parse().unwrap();
            assert!(!announced_over_mdns(false, bound), "{bound}");
            assert_eq!(mdns_off_notice(false, bound), None, "{bound}");
        }
    }
}

#[cfg(all(test, feature = "mdns"))]
mod record_tests {
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

    /// Regression (#729): a wildcard bind was announced with one address
    /// picked from the machine's interfaces, so the record reached that
    /// address's network only.
    #[test]
    fn a_wildcard_bind_is_announced_with_every_interface_address() {
        for wildcard in ["0.0.0.0:8431", "[::]:8431"] {
            let bound: SocketAddr = wildcard.parse().unwrap();
            let info = service_info(
                &kit_role(Some("Plans the work")),
                bound,
                || IpAddr::V4(Ipv4Addr::new(10, 0, 0, 140)),
                None,
                &[],
            )
            .unwrap();

            assert!(info.is_addr_auto(), "{wildcard}");
            assert!(info.get_addresses().is_empty(), "{wildcard}");
            assert_eq!(info.get_port(), 8431);
            assert_eq!(info.get_fullname(), "planner._a2a._tcp.local.");
            assert_eq!(info.get_property_val_str("ip"), Some("10.0.0.140"));
        }
    }

    #[test]
    fn a_bind_to_one_address_is_announced_with_that_address() {
        let bound: SocketAddr = "10.0.0.140:8431".parse().unwrap();
        let info = service_info(
            &kit_role(None),
            bound,
            || panic!("a bind to one address needs no LAN lookup"),
            None,
            &[],
        )
        .unwrap();

        assert!(!info.is_addr_auto());
        let addresses: Vec<_> = info.get_addresses().iter().copied().collect();
        assert_eq!(addresses, vec![bound.ip()]);
        assert_eq!(info.get_property_val_str("ip"), Some("10.0.0.140"));
    }
}

#[cfg(all(test, feature = "mdns"))]
mod departure_tests {
    use super::*;
    use mdns_sd::{IfKind, ServiceDaemon, ServiceEvent, ServiceInfo};
    use std::net::Ipv4Addr;
    use std::time::{Duration, Instant};

    const SERVICE_TYPE: &str = "_a2a._tcp.local.";

    /// A daemon on the loopback interfaces only, so nothing the test sends
    /// leaves the machine.
    fn loopback_daemon() -> ServiceDaemon {
        let daemon = ServiceDaemon::new().expect("mDNS daemon");
        daemon
            .disable_interface(IfKind::All)
            .expect("leave every interface");
        daemon
            .enable_interface(vec![IfKind::LoopbackV4, IfKind::LoopbackV6])
            .expect("join the loopback interfaces");
        daemon
    }

    fn wait_for(
        events: &mdns_sd::Receiver<ServiceEvent>,
        what: impl Fn(&ServiceEvent) -> bool,
    ) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match events.recv_timeout(left) {
                Ok(event) if what(&event) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
        false
    }

    /// Regression: an agent that shut down cleanly sent no goodbye, so a
    /// peer never saw it leave and refused the gossip key it restarted with.
    #[test]
    fn a_departing_agent_is_seen_to_leave() {
        let agent_id = format!("departure-{}", std::process::id());
        let agent = loopback_daemon();
        let service = ServiceInfo::new(
            SERVICE_TYPE,
            &agent_id,
            &format!("{agent_id}.local."),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            43_210,
            HashMap::from([("agent_id".to_string(), agent_id.clone())]),
        )
        .expect("service info");
        let fullname = service.get_fullname().to_string();
        agent.register(service).expect("register the agent");

        let peer = loopback_daemon();
        let events = peer.browse(SERVICE_TYPE).expect("browse");
        assert!(
            wait_for(
                &events,
                |e| matches!(e, ServiceEvent::ServiceResolved(info) if info.get_fullname() == fullname)
            ),
            "the peer never saw the agent arrive"
        );

        announce_departure(&agent, &fullname);
        assert!(
            wait_for(
                &events,
                |e| matches!(e, ServiceEvent::ServiceRemoved(_, name) if *name == fullname)
            ),
            "the peer never saw the agent leave"
        );

        let _ = peer.shutdown();
        let _ = agent.shutdown();
    }
}
