//! Listen-address policy for the agent's A2A RPC endpoint.
//!
//! The RPC surface does not authenticate its callers, so the address it
//! listens on is the only thing deciding who can reach it. Everything here
//! therefore fails closed: an address that cannot be understood stops
//! startup instead of being replaced by a guess.

use std::net::{IpAddr, SocketAddr};

/// Listen address used when the kit has no `runtime.listen`, and for the
/// zero-config default: this machine only, on a port the OS picks.
///
/// Listening on any other interface has to be written into the kit. A
/// default is what runs when nobody made a decision, and an endpoint that
/// accepts unauthenticated calls from the network must be somebody's
/// decision.
pub(crate) const DEFAULT_LISTEN: &str = "127.0.0.1:0";

/// The rate limit the agent runs its RPC endpoint with. Defined once so the
/// security audit reports the limit the agent applies, not one of its own.
pub(crate) fn rpc_rate_limit() -> arkavo_protocol::rate_limit::RateLimitConfig {
    arkavo_protocol::rate_limit::RateLimitConfig::default()
}

/// Whether only processes on this machine can reach `ip`.
///
/// An IPv4-mapped IPv6 address is judged by the IPv4 address it carries, so
/// `::ffff:127.0.0.1` counts as loopback like `127.0.0.1` does.
pub(crate) fn is_loopback(ip: IpAddr) -> bool {
    ip.to_canonical().is_loopback()
}

/// The warning to show at startup when `bound` can be reached from another
/// machine, `None` when it is loopback.
pub(crate) fn exposure_warning(bound: SocketAddr) -> Option<String> {
    if is_loopback(bound.ip()) {
        return None;
    }
    let reach = if bound.ip().is_unspecified() {
        "every network interface of this machine".to_string()
    } else {
        format!("the network interface holding {}", bound.ip())
    };
    Some(format!(
        "WARNING: the agent RPC endpoint is listening on {bound}, which is {reach}.\n\
         WARNING: the RPC endpoint is unauthenticated: any host that can reach this address \
         can call its methods.\n\
         WARNING: set runtime.listen to \"{DEFAULT_LISTEN}\" in the kit to accept \
         connections from this machine only."
    ))
}

/// Parse a kit's `runtime.listen` (or the zero-config default) into the
/// socket address to bind.
///
/// Goes through [`SocketAddr`] rather than splitting on `:` so a bracketed
/// IPv6 literal such as `[::1]:8080` is read as one host. Hostnames are
/// rejected: resolving one can yield several addresses, and which of them the
/// endpoint ends up on must not depend on the resolver.
pub(crate) fn parse_listen(listen: &str) -> Result<SocketAddr, String> {
    listen.trim().parse::<SocketAddr>().map_err(|_| {
        format!(
            "Invalid listen address {listen:?}: expected an IP address and port, \
             for example 127.0.0.1:8080 or [::1]:8080"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn parses_ipv4_with_port() {
        assert_eq!(
            parse_listen("127.0.0.1:8080").unwrap(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080)
        );
        assert_eq!(
            parse_listen("0.0.0.0:0").unwrap(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)
        );
    }

    /// Regression: splitting on `:` saw more than two parts in a bracketed
    /// IPv6 address and reported it as invalid.
    #[test]
    fn parses_bracketed_ipv6_with_port() {
        assert_eq!(
            parse_listen("[::1]:8080").unwrap(),
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 8080)
        );
        assert_eq!(
            parse_listen("[::]:0").unwrap(),
            SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0)
        );
    }

    #[test]
    fn surrounding_whitespace_is_ignored() {
        assert_eq!(
            parse_listen(" 127.0.0.1:9000\n").unwrap(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9000)
        );
    }

    #[test]
    fn the_default_listen_address_is_loopback() {
        let default = parse_listen(DEFAULT_LISTEN).unwrap();
        assert!(is_loopback(default.ip()));
        assert_eq!(default.port(), 0);
        assert_eq!(exposure_warning(default), None);
    }

    #[test]
    fn loopback_addresses_are_not_warned_about() {
        for local in ["127.0.0.1:8080", "127.8.9.10:1", "[::1]:8080"] {
            assert_eq!(exposure_warning(parse_listen(local).unwrap()), None);
        }
        assert!(is_loopback("::ffff:127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn network_reachable_addresses_are_warned_about() {
        for exposed in [
            "0.0.0.0:8431",
            "[::]:8431",
            "10.0.0.140:8431",
            "[fe80::1]:8431",
            "[::ffff:10.0.0.140]:8431",
        ] {
            let bound = parse_listen(exposed).unwrap();
            let warning = exposure_warning(bound)
                .unwrap_or_else(|| panic!("{exposed} is reachable from the network"));
            assert!(warning.contains("unauthenticated"), "{warning}");
            assert!(warning.contains(&bound.to_string()), "{warning}");
            assert!(warning.contains("runtime.listen"), "{warning}");
        }
    }

    #[test]
    fn rejects_anything_that_is_not_an_ip_and_port() {
        for bad in [
            "",
            "8080",
            "127.0.0.1",
            "::1",
            "[::1]",
            "localhost:8080",
            "127.0.0.1:notaport",
            "127.0.0.1:99999",
            "127.0.0.1:8080:9090",
            "http://127.0.0.1:8080",
        ] {
            let err = parse_listen(bad).expect_err(bad);
            assert!(
                err.contains("Invalid listen address"),
                "{bad:?} gave an unexpected error: {err}"
            );
        }
    }
}
