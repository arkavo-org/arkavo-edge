//! Listen-address policy for the agent's A2A RPC endpoint.
//!
//! The RPC surface does not authenticate its callers, so the address it
//! listens on is the only thing deciding who can reach it. An address that
//! cannot be understood stops startup instead of being replaced by a guess,
//! an agent that other machines can reach says so when it starts, and
//! `--trust` keeps an agent on this machine whatever the kit asks for.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// Listen address used when the kit has no `runtime.listen`, and for the
/// zero-config default: every interface, on a port the OS picks.
///
/// An agent started with no configuration is meant to be found and reached
/// by agents on other devices. The endpoint does not authenticate callers,
/// so such a start prints [`exposure_warning`], and `--trust` moves the agent
/// to loopback ([`trusted_listen`]).
pub(crate) const DEFAULT_LISTEN: &str = "0.0.0.0:0";

/// A listen address for this machine only, on a port the OS picks. Shown to
/// the operator as the value to pin in a kit.
pub(crate) const LOOPBACK_LISTEN: &str = "127.0.0.1:0";

/// The host a `--trust` run is moved to.
const TRUSTED_HOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

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

/// The notice to show at startup when `bound` can be reached from another
/// machine, `None` when it is loopback.
///
/// This is what an ordinary start prints, so it is two lines: what is true
/// of the endpoint, and the one flag that changes it.
pub(crate) fn exposure_warning(bound: SocketAddr) -> Option<String> {
    if is_loopback(bound.ip()) {
        return None;
    }
    Some(format!(
        "Agent RPC endpoint {bound} is not authenticated and is reachable from the network.\n\
         Start the agent with --trust to keep it on this machine."
    ))
}

/// Where a `--trust` run listens.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TrustedListen {
    /// The address to bind. Always loopback.
    pub addr: SocketAddr,
    /// What to tell the operator when `--trust` set aside an address the
    /// kit asked for. `None` when the kit asked for nothing `--trust`
    /// had to change.
    pub notice: Option<String>,
}

/// Apply `--trust` to `resolved`, the address the agent would listen on
/// without it. `kit_listen` is the kit's `runtime.listen` as written, `None`
/// when the address is the built-in default.
///
/// The port is kept: it is what `runtime.listen` or `-p` selected. A
/// loopback host is kept as written, so a kit that names `[::1]` stays on
/// IPv6. Any other host becomes `127.0.0.1`.
pub(crate) fn trusted_listen(resolved: SocketAddr, kit_listen: Option<&str>) -> TrustedListen {
    if is_loopback(resolved.ip()) {
        return TrustedListen {
            addr: resolved,
            notice: None,
        };
    }
    let addr = SocketAddr::new(TRUSTED_HOST, resolved.port());
    let notice = kit_listen.map(|kit_listen| {
        format!(
            "--trust is keeping the agent on loopback: it listens on {addr}, not on {} \
             as runtime.listen in the kit asks.",
            kit_listen.trim()
        )
    });
    TrustedListen { addr, notice }
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
    fn the_default_listen_address_is_every_interface() {
        let default = parse_listen(DEFAULT_LISTEN).unwrap();
        assert_eq!(default.ip(), IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        assert_eq!(default.port(), 0);
        assert!(exposure_warning(default).is_some());
    }

    #[test]
    fn the_address_offered_for_pinning_is_loopback() {
        let pinned = parse_listen(LOOPBACK_LISTEN).unwrap();
        assert!(is_loopback(pinned.ip()));
        assert_eq!(pinned.port(), 0);
        assert_eq!(exposure_warning(pinned), None);
    }

    fn addr(listen: &str) -> SocketAddr {
        parse_listen(listen).unwrap()
    }

    #[test]
    fn trust_moves_the_default_to_loopback_without_a_notice() {
        assert_eq!(
            trusted_listen(addr(DEFAULT_LISTEN), None),
            TrustedListen {
                addr: addr("127.0.0.1:0"),
                notice: None,
            }
        );
    }

    #[test]
    fn trust_keeps_the_selected_port() {
        assert_eq!(
            trusted_listen(addr("0.0.0.0:8343"), None).addr,
            addr("127.0.0.1:8343")
        );
        assert_eq!(
            trusted_listen(addr("[::]:8343"), Some("[::]:8080")).addr,
            addr("127.0.0.1:8343")
        );
    }

    #[test]
    fn trust_overrides_a_network_address_from_the_kit_and_names_it() {
        for kit_listen in [
            "0.0.0.0:8342",
            "10.0.0.140:8342",
            "[::]:8342",
            "[fe80::1]:8342",
        ] {
            let trusted = trusted_listen(addr(kit_listen), Some(kit_listen));

            assert_eq!(trusted.addr, addr("127.0.0.1:8342"), "{kit_listen}");
            let notice = trusted
                .notice
                .unwrap_or_else(|| panic!("overriding {kit_listen} must be said"));
            assert_eq!(notice.lines().count(), 1, "{notice}");
            assert!(notice.contains("--trust"), "{notice}");
            assert!(notice.contains("loopback"), "{notice}");
            assert!(notice.contains(kit_listen), "{notice}");
            assert!(notice.contains("127.0.0.1:8342"), "{notice}");
        }
    }

    #[test]
    fn trust_keeps_a_loopback_address_from_the_kit_as_written() {
        for kit_listen in ["127.0.0.1:8342", "127.8.9.10:8342", "[::1]:8342"] {
            assert_eq!(
                trusted_listen(addr(kit_listen), Some(kit_listen)),
                TrustedListen {
                    addr: addr(kit_listen),
                    notice: None,
                },
                "{kit_listen}"
            );
        }
    }

    #[test]
    fn a_trusted_address_is_never_warned_about() {
        for resolved in ["0.0.0.0:0", "[::]:9", "10.0.0.140:8342", "[::1]:8342"] {
            let trusted = trusted_listen(addr(resolved), Some(resolved));
            assert!(is_loopback(trusted.addr.ip()), "{resolved}");
            assert_eq!(exposure_warning(trusted.addr), None, "{resolved}");
        }
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
            assert!(warning.contains("not authenticated"), "{warning}");
            assert!(warning.contains("reachable from the network"), "{warning}");
            assert!(warning.contains(&bound.to_string()), "{warning}");
            assert!(warning.contains("--trust"), "{warning}");
            assert_eq!(warning.lines().count(), 2, "{warning}");
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
