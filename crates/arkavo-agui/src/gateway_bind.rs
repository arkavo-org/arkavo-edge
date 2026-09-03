//! Bind address resolution for the AG-UI gateway.
//!
//! The gateway serves an unauthenticated control surface (run agents, read
//! config, stream telemetry), so it must not listen on a routable interface
//! by default. The bind address defaults to loopback; set `ARKAVO_AGUI_BIND`
//! to an IP (e.g. `0.0.0.0`) to explicitly expose it for remote browser
//! access in development.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

pub const BIND_ENV_VAR: &str = "ARKAVO_AGUI_BIND";

/// Resolve the gateway bind address for `port`, honoring `ARKAVO_AGUI_BIND`.
pub fn resolve_bind_addr(port: u16) -> SocketAddr {
    parse_bind_addr(std::env::var(BIND_ENV_VAR).ok().as_deref(), port)
}

/// Pure resolution logic, kept separate from env access for testability.
fn parse_bind_addr(value: Option<&str>, port: u16) -> SocketAddr {
    let Some(raw) = value else {
        return loopback(port);
    };
    match raw.trim().parse::<IpAddr>() {
        Ok(ip) => SocketAddr::new(ip, port),
        Err(_) => {
            // Fail closed: an invalid override must never widen the bind.
            eprintln!("AG-UI: invalid {BIND_ENV_VAR} value '{raw}', falling back to 127.0.0.1");
            loopback(port)
        }
    }
}

fn loopback(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bind_is_loopback() {
        let addr = parse_bind_addr(None, 7700);
        assert!(addr.ip().is_loopback());
        assert_eq!(addr.port(), 7700);
    }

    #[test]
    fn explicit_wildcard_bind_is_honored() {
        let addr = parse_bind_addr(Some("0.0.0.0"), 7700);
        assert_eq!(addr.ip(), IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        assert_eq!(addr.port(), 7700);
    }

    #[test]
    fn explicit_specific_bind_is_honored() {
        let addr = parse_bind_addr(Some("192.168.1.10"), 8080);
        assert_eq!(addr.ip(), IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)));
        assert_eq!(addr.port(), 8080);
    }

    #[test]
    fn invalid_bind_falls_back_to_loopback() {
        let addr = parse_bind_addr(Some("not-an-ip"), 7700);
        assert!(addr.ip().is_loopback());
    }

    #[test]
    #[serial_test::serial(env_arkavo_agui_bind)]
    fn env_override_resolution() {
        let prev = std::env::var(BIND_ENV_VAR).ok();
        // SAFETY: serial_test serializes any test in this binary that
        // touches the same key, so set_var/remove_var calls do not race.
        unsafe {
            std::env::set_var(BIND_ENV_VAR, "0.0.0.0");
        }
        let addr = resolve_bind_addr(7700);
        unsafe {
            match prev {
                Some(v) => std::env::set_var(BIND_ENV_VAR, v),
                None => std::env::remove_var(BIND_ENV_VAR),
            }
        }
        assert_eq!(addr.ip(), IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    }
}
