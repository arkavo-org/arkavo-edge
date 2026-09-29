//! Listen-address policy for the agent's A2A RPC endpoint.
//!
//! The RPC surface does not authenticate its callers, so the address it
//! listens on is the only thing deciding who can reach it. Everything here
//! therefore fails closed: an address that cannot be understood stops
//! startup instead of being replaced by a guess.

use std::net::SocketAddr;

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
