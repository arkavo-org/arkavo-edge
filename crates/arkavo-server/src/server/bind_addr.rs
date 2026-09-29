//! The address the A2A RPC endpoint binds, and how that address is written
//! when it is handed to a client.

use std::net::{IpAddr, SocketAddr};

use arkavo_protocol::error::{A2aError, Result};

/// The socket address a `bind_address` / `port` pair stands for.
///
/// Built from the parsed IP instead of parsing `"{bind_address}:{port}"`:
/// joined that way an IPv6 host is ambiguous (`::1:8080` is itself a valid
/// IPv6 address, with no port), so an IPv6 bind could never start.
pub(super) fn bind_socket_addr(bind_address: &str, port: u16) -> Result<SocketAddr> {
    let host = bind_address.trim();
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let ip: IpAddr = host.parse().map_err(|e| {
        A2aError::InvalidEndpoint(format!("Invalid bind address {bind_address:?}: {e}"))
    })?;
    Ok(SocketAddr::new(ip, port))
}

/// The `http://` URL of an endpoint at `addr`, with an IPv6 host in brackets.
pub(super) fn endpoint_url(addr: SocketAddr) -> String {
    format!("http://{addr}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn ipv4_bind_address_keeps_its_port() {
        assert_eq!(
            bind_socket_addr("127.0.0.1", 8080).unwrap(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080)
        );
    }

    /// Regression: `"::1"` joined to a port with `:` parsed as a bare IPv6
    /// address, so the server refused to start on any IPv6 bind address.
    #[test]
    fn ipv6_bind_address_is_accepted_with_or_without_brackets() {
        let expected = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 8080);
        assert_eq!(bind_socket_addr("::1", 8080).unwrap(), expected);
        assert_eq!(bind_socket_addr("[::1]", 8080).unwrap(), expected);
    }

    #[test]
    fn a_hostname_or_empty_bind_address_is_an_error() {
        for bad in ["", "localhost", "127.0.0.1:8080", "[::1"] {
            assert!(bind_socket_addr(bad, 8080).is_err(), "{bad:?} must fail");
        }
    }

    #[test]
    fn endpoint_url_brackets_an_ipv6_host() {
        assert_eq!(
            endpoint_url(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8431)),
            "http://127.0.0.1:8431"
        );
        assert_eq!(
            endpoint_url(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 8431)),
            "http://[::1]:8431"
        );
    }
}
