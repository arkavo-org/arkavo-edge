//! Listen-address policy for the agent's A2A RPC endpoint.
//!
//! The RPC surface does not authenticate its callers, so the address it
//! listens on is the only thing deciding who can reach it. An address that
//! cannot be understood stops startup instead of being replaced by a guess,
//! an agent that other machines can reach says so when it starts, and
//! `--bind` chooses the address whatever the kit asks for.

use std::net::{IpAddr, SocketAddr};

/// Listen address used when the kit has no `runtime.listen`, and for the
/// zero-config default: every interface, on a port the OS picks.
///
/// An agent started with no configuration is meant to be found and reached
/// by agents on other devices. The endpoint does not authenticate callers,
/// so such a start prints [`exposure_warning`], and `--bind` moves the agent
/// to an address of the operator's choosing ([`bind_listen`]).
pub(crate) const DEFAULT_LISTEN: &str = "0.0.0.0:0";

/// A listen address for this machine only, on a port the OS picks. Shown to
/// the operator as the value to pin in a kit.
pub(crate) const LOOPBACK_LISTEN: &str = "127.0.0.1:0";

/// The `--bind` value that keeps an agent on this machine. Shown to the
/// operator wherever loopback is the advice.
pub(crate) const LOOPBACK_BIND: &str = "--bind 127.0.0.1";

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
         Start the agent with {LOOPBACK_BIND} to keep it on this machine."
    ))
}

/// The value of `--bind`: the host to listen on, and the port when the
/// operator named one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindAddress {
    pub ip: IpAddr,
    pub port: Option<u16>,
}

/// Parse the value of `--bind`: an IP address on its own (`127.0.0.1`,
/// `::1`, `[::1]`) or with a port (`127.0.0.1:8342`, `[::1]:8342`).
///
/// An address with a port goes through [`parse_listen`], so it is read the
/// way `runtime.listen` is. Hostnames are rejected for the same reason as
/// there: which address the endpoint ends up on must not depend on the
/// resolver.
pub(crate) fn parse_bind(bind: &str) -> Result<BindAddress, String> {
    if let Ok(addr) = parse_listen(bind) {
        return Ok(BindAddress {
            ip: addr.ip(),
            port: Some(addr.port()),
        });
    }
    let bare = bind.trim();
    let unbracketed = bare
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(bare);
    unbracketed
        .parse::<IpAddr>()
        .map(|ip| BindAddress { ip, port: None })
        .map_err(|_| {
            format!(
                "Invalid bind address {bind:?}: expected an IP address, with or without a port, \
                 for example 127.0.0.1, 127.0.0.1:8342 or [::1]:8342"
            )
        })
}

/// Where a run started with `--bind` listens.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BindListen {
    /// The address to bind.
    pub addr: SocketAddr,
    /// What to tell the operator when `--bind` set aside an address the kit
    /// asked for. `None` when the kit asked for nothing `--bind` changed.
    pub notice: Option<String>,
}

/// Apply `--bind` to `resolved`, the address the agent would listen on
/// without it, with `-p` already applied. `kit_listen` is the kit's
/// `runtime.listen` as written, `None` when the address is the built-in
/// default.
///
/// The host is always the one `--bind` names. The port is the one `--bind`
/// names when it names one; otherwise it stays what `-p`, `runtime.listen`
/// or the default selected. The notice is owed only for what `--bind`
/// itself changed: a port that `-p` moved is not reported here, as it is
/// not reported without `--bind`.
pub(crate) fn bind_listen(
    resolved: SocketAddr,
    bind: BindAddress,
    kit_listen: Option<&str>,
) -> BindListen {
    let addr = SocketAddr::new(bind.ip, bind.port.unwrap_or_else(|| resolved.port()));
    let notice = kit_listen
        .filter(|kit_listen| {
            parse_listen(kit_listen).is_ok_and(|kit_addr| {
                addr.ip() != kit_addr.ip() || bind.port.is_some_and(|port| port != kit_addr.port())
            })
        })
        .map(|kit_listen| {
            format!(
                "--bind is choosing the listen address: the agent listens on {addr}, not on {} \
                 as runtime.listen in the kit asks.",
                kit_listen.trim()
            )
        });
    BindListen { addr, notice }
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

    fn bind(bind: &str) -> BindAddress {
        parse_bind(bind).unwrap()
    }

    #[test]
    fn a_bind_with_a_port_is_read_like_a_listen_address() {
        assert_eq!(
            bind("127.0.0.1:8342"),
            BindAddress {
                ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port: Some(8342),
            }
        );
        assert_eq!(
            bind("[::1]:8342"),
            BindAddress {
                ip: IpAddr::V6(Ipv6Addr::LOCALHOST),
                port: Some(8342),
            }
        );
        assert_eq!(
            bind(" 0.0.0.0:0\n"),
            BindAddress {
                ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                port: Some(0),
            }
        );
    }

    #[test]
    fn a_bind_without_a_port_names_only_the_host() {
        assert_eq!(
            bind("127.0.0.1"),
            BindAddress {
                ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port: None,
            }
        );
        assert_eq!(
            bind("0.0.0.0"),
            BindAddress {
                ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                port: None,
            }
        );
        for v6 in ["::1", "[::1]", " [::1] "] {
            assert_eq!(
                bind(v6),
                BindAddress {
                    ip: IpAddr::V6(Ipv6Addr::LOCALHOST),
                    port: None,
                },
                "{v6:?}"
            );
        }
    }

    #[test]
    fn a_bind_that_is_not_an_ip_address_is_rejected() {
        for bad in [
            "",
            "8342",
            ":8342",
            "localhost",
            "localhost:8342",
            "127.0.0.1:notaport",
            "127.0.0.1:99999",
            "[::1",
            "http://127.0.0.1:8342",
        ] {
            let err = parse_bind(bad).expect_err(bad);
            assert!(
                err.contains("Invalid bind address"),
                "{bad:?} gave an unexpected error: {err}"
            );
            assert!(err.contains("127.0.0.1"), "{err}");
        }
    }

    #[test]
    fn bind_moves_the_default_to_the_named_host_without_a_notice() {
        assert_eq!(
            bind_listen(addr(DEFAULT_LISTEN), bind("127.0.0.1"), None),
            BindListen {
                addr: addr("127.0.0.1:0"),
                notice: None,
            }
        );
        assert_eq!(
            bind_listen(addr(DEFAULT_LISTEN), bind("[::1]"), None).addr,
            addr("[::1]:0")
        );
    }

    /// The port of `resolved` is what `-p` or `runtime.listen` selected;
    /// `--bind` without a port keeps it.
    #[test]
    fn bind_without_a_port_keeps_the_selected_port() {
        assert_eq!(
            bind_listen(addr("0.0.0.0:8343"), bind("127.0.0.1"), None).addr,
            addr("127.0.0.1:8343")
        );
        assert_eq!(
            bind_listen(addr("[::]:8343"), bind("127.0.0.1"), Some("[::]:8343")).addr,
            addr("127.0.0.1:8343")
        );
    }

    #[test]
    fn bind_with_a_port_uses_that_port() {
        assert_eq!(
            bind_listen(addr("0.0.0.0:8343"), bind("127.0.0.1:8342"), None).addr,
            addr("127.0.0.1:8342")
        );
        assert_eq!(
            bind_listen(addr(DEFAULT_LISTEN), bind("[::1]:8342"), None).addr,
            addr("[::1]:8342")
        );
    }

    #[test]
    fn bind_overrides_an_address_from_the_kit_and_names_it() {
        for (kit_listen, bind_to, listens_on) in [
            ("0.0.0.0:8342", "127.0.0.1", "127.0.0.1:8342"),
            ("10.0.0.140:8342", "127.0.0.1", "127.0.0.1:8342"),
            ("[::]:8342", "[::1]", "[::1]:8342"),
            ("127.0.0.1:8342", "0.0.0.0", "0.0.0.0:8342"),
            ("127.0.0.1:8342", "127.0.0.1:9000", "127.0.0.1:9000"),
        ] {
            let chosen = bind_listen(addr(kit_listen), bind(bind_to), Some(kit_listen));

            assert_eq!(chosen.addr, addr(listens_on), "{kit_listen} {bind_to}");
            let notice = chosen
                .notice
                .unwrap_or_else(|| panic!("overriding {kit_listen} with {bind_to} must be said"));
            assert_eq!(notice.lines().count(), 1, "{notice}");
            assert!(notice.contains("--bind"), "{notice}");
            assert!(notice.contains("runtime.listen"), "{notice}");
            assert!(notice.contains(kit_listen), "{notice}");
            assert!(notice.contains(listens_on), "{notice}");
        }
    }

    /// A `--bind` that asks for what the kit already has changes nothing,
    /// so there is nothing to tell the operator.
    #[test]
    fn bind_that_matches_the_kit_gives_no_notice() {
        for (kit_listen, bind_to) in [
            ("127.0.0.1:8342", "127.0.0.1"),
            ("127.0.0.1:8342", "127.0.0.1:8342"),
            ("[::1]:8342", "::1"),
            ("0.0.0.0:8342", "0.0.0.0"),
        ] {
            assert_eq!(
                bind_listen(addr(kit_listen), bind(bind_to), Some(kit_listen)),
                BindListen {
                    addr: addr(kit_listen),
                    notice: None,
                },
                "{kit_listen} {bind_to}"
            );
        }
    }

    /// `-p` moved the port before `--bind` was applied. That move is not
    /// reported, with or without `--bind`: only a host `--bind` changed is.
    #[test]
    fn bind_does_not_report_a_port_that_p_moved() {
        let chosen = bind_listen(
            addr("127.0.0.1:9000"),
            bind("127.0.0.1"),
            Some("127.0.0.1:8342"),
        );
        assert_eq!(chosen.addr, addr("127.0.0.1:9000"));
        assert_eq!(chosen.notice, None);
    }

    #[test]
    fn a_loopback_bind_is_never_warned_about() {
        for resolved in ["0.0.0.0:0", "[::]:9", "10.0.0.140:8342"] {
            for bind_to in ["127.0.0.1", "[::1]", "127.0.0.1:8342"] {
                let chosen = bind_listen(addr(resolved), bind(bind_to), Some(resolved));
                assert!(is_loopback(chosen.addr.ip()), "{resolved} {bind_to}");
                assert_eq!(exposure_warning(chosen.addr), None, "{resolved} {bind_to}");
            }
        }
    }

    #[test]
    fn a_network_bind_is_warned_about() {
        for bind_to in ["0.0.0.0", "[::]", "10.0.0.140:8342"] {
            let chosen = bind_listen(
                addr("127.0.0.1:8342"),
                bind(bind_to),
                Some("127.0.0.1:8342"),
            );
            assert!(exposure_warning(chosen.addr).is_some(), "{bind_to}");
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
            assert!(warning.contains("--bind 127.0.0.1"), "{warning}");
            assert!(!warning.contains("--trust"), "{warning}");
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
