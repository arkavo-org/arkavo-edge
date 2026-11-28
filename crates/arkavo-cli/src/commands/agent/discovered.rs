//! The peers mDNS has resolved, and where (INGRESS-004).

use std::collections::HashMap;
use std::net::IpAddr;

/// The peers discovery has reported to gossip, by agent id, with the
/// address each was last reported at.
#[derive(Debug, Default)]
pub(super) struct DiscoveredPeers {
    addresses: HashMap<String, Option<String>>,
}

impl DiscoveredPeers {
    /// Record that `agent_id` resolved at `address`. True when gossip must
    /// hear of it: a peer seen for the first time, or one that came back at
    /// another address, as a peer that crashed and restarted on a new port
    /// does without a goodbye. A resolution without an address keeps the one
    /// already known.
    pub(super) fn resolved(&mut self, agent_id: &str, address: Option<String>) -> bool {
        match self.addresses.get(agent_id) {
            Some(known) if address.is_none() || *known == address => false,
            _ => {
                self.addresses.insert(agent_id.to_string(), address);
                true
            }
        }
    }

    /// Forget `agent_id`. True when it had been reported, so its departure
    /// must be too.
    pub(super) fn removed(&mut self, agent_id: &str) -> bool {
        self.addresses.remove(agent_id).is_some()
    }
}

/// The address to reach a peer at, the same one every time it resolves
/// with the same addresses: an announcement lists them in no fixed order,
/// and taking whichever came first would read as the peer moving.
pub(super) fn peer_address<'a>(
    addresses: impl IntoIterator<Item = &'a IpAddr>,
    port: u16,
) -> Option<String> {
    addresses
        .into_iter()
        .min_by_key(|ip| (ip.is_ipv6(), **ip))
        .map(|ip| match ip {
            IpAddr::V4(v4) => format!("http://{v4}:{port}"),
            IpAddr::V6(v6) => format!("http://[{v6}]:{port}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    const FIRST: &str = "http://10.0.0.140:64380";
    const RESTARTED: &str = "http://10.0.0.140:64417";

    /// Regression: a peer that crashed and came back on a new port was never
    /// reported again, so gossip kept sending to the dead port.
    #[spec("INGRESS-004")]
    #[test]
    fn a_peer_back_at_a_new_address_is_reported_again() {
        let mut peers = DiscoveredPeers::default();
        assert!(peers.resolved("beta", Some(FIRST.into())));
        assert!(!peers.resolved("beta", Some(FIRST.into())));
        assert!(peers.resolved("beta", Some(RESTARTED.into())));
        assert!(!peers.resolved("beta", Some(RESTARTED.into())));
    }

    #[spec("INGRESS-004")]
    #[test]
    fn a_resolution_without_an_address_keeps_the_known_one() {
        let mut peers = DiscoveredPeers::default();
        assert!(peers.resolved("beta", Some(FIRST.into())));
        assert!(!peers.resolved("beta", None));
        assert!(!peers.resolved("beta", Some(FIRST.into())));
    }

    #[spec("INGRESS-004")]
    #[test]
    fn a_departure_is_reported_once_and_a_return_is_new() {
        let mut peers = DiscoveredPeers::default();
        assert!(!peers.removed("beta"));
        peers.resolved("beta", Some(FIRST.into()));
        assert!(peers.removed("beta"));
        assert!(!peers.removed("beta"));
        assert!(peers.resolved("beta", Some(FIRST.into())));
    }

    #[test]
    fn the_same_addresses_in_any_order_give_the_same_address() {
        let v4: IpAddr = "10.0.0.140".parse().unwrap();
        let other: IpAddr = "192.168.1.9".parse().unwrap();
        let v6: IpAddr = "fe80::1".parse().unwrap();
        let expected = Some("http://10.0.0.140:8342".to_string());
        assert_eq!(peer_address(&[v6, other, v4], 8342), expected);
        assert_eq!(peer_address(&[v4, v6, other], 8342), expected);
        assert_eq!(
            peer_address(&[v6], 8342),
            Some("http://[fe80::1]:8342".to_string())
        );
        assert_eq!(peer_address(&[], 8342), None);
    }
}
