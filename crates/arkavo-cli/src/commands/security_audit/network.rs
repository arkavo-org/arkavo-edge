//! Audit checks for the agent's A2A RPC endpoint.
//!
//! Every check here reads the configuration `arkavo agent` would run with
//! in the audited directory: the kit discovery finds, and the defaults the
//! agent falls back to. A control the endpoint does not have is reported as
//! missing; nothing passes on an assumption.

use std::net::SocketAddr;
use std::path::Path;

use arkavo_protocol::rate_limit::RateLimitConfig;
use arkavo_swarmkit::DiscoverError;

use super::{AuditResult, AuditStatus, result};
use crate::commands::agent::listen::{DEFAULT_LISTEN, LOOPBACK_LISTEN, is_loopback, parse_listen};

const NETWORK: &str = "Network";
const AUTHENTICATION: &str = "Authentication";

/// The address the RPC endpoint would listen on.
pub(super) enum Endpoint {
    /// The agent would bind `addr`; `origin` says where the address is set.
    Bound { addr: SocketAddr, origin: String },
    /// The configured address does not parse, so the agent would not start.
    Invalid { reason: String },
    /// The configuration could not be read.
    Unknown { reason: String },
}

/// Resolve the listen address the way `arkavo agent` does when started in
/// `cwd` without flags: the discovered kit's `runtime.listen`, else the
/// built-in default.
pub(super) fn effective_endpoint(cwd: &Path) -> Endpoint {
    let (listen, origin) = match arkavo_swarmkit::discover_kit_path(cwd) {
        Ok(path) => match arkavo_swarmkit::load_kit_file(&path) {
            Ok(kit) => match kit.config.runtime.listen {
                Some(listen) => (listen, format!("runtime.listen in {}", path.display())),
                None => (
                    DEFAULT_LISTEN.to_string(),
                    format!(
                        "built-in default, {} sets no runtime.listen",
                        path.display()
                    ),
                ),
            },
            Err(e) => {
                return Endpoint::Unknown {
                    reason: format!("the kit could not be loaded: {e}"),
                };
            }
        },
        Err(DiscoverError::NotFound | DiscoverError::AgentsMdUnsupported { .. }) => (
            DEFAULT_LISTEN.to_string(),
            "built-in default, no kit found".to_string(),
        ),
        Err(e) => {
            return Endpoint::Unknown {
                reason: format!("kit discovery failed: {e}"),
            };
        }
    };

    match parse_listen(&listen) {
        Ok(addr) => Endpoint::Bound { addr, origin },
        Err(reason) => Endpoint::Invalid {
            reason: format!("{reason} ({origin})"),
        },
    }
}

pub(super) fn check_bind(endpoint: &Endpoint) -> AuditResult {
    let name = "Bind address";
    match endpoint {
        Endpoint::Bound { addr, origin } if is_loopback(addr.ip()) => result(
            name,
            NETWORK,
            AuditStatus::Pass,
            format!("RPC endpoint listens on {addr}, this machine only ({origin})"),
        ),
        Endpoint::Bound { addr, origin } => result(
            name,
            NETWORK,
            AuditStatus::Fail,
            format!(
                "RPC endpoint listens on {addr}, reachable from the network ({origin}); \
                 start the agent with --trust, or set runtime.listen to \"{LOOPBACK_LISTEN}\", \
                 to keep it on this machine"
            ),
        ),
        Endpoint::Invalid { reason } => result(
            name,
            NETWORK,
            AuditStatus::Fail,
            format!("{reason}; the agent will not start"),
        ),
        Endpoint::Unknown { reason } => result(
            name,
            NETWORK,
            AuditStatus::Warn,
            format!("Listen address could not be determined: {reason}"),
        ),
    }
}

/// The endpoint is served over plain TCP: the server has no TLS
/// configuration to inspect. That is only acceptable while the traffic
/// cannot leave the machine.
pub(super) fn check_transport(endpoint: &Endpoint) -> AuditResult {
    let name = "Transport encryption";
    match endpoint {
        Endpoint::Bound { addr, .. } if is_loopback(addr.ip()) => result(
            name,
            NETWORK,
            AuditStatus::Pass,
            format!(
                "RPC endpoint has no TLS; it listens on {addr}, so its traffic stays on this machine"
            ),
        ),
        Endpoint::Bound { addr, .. } => result(
            name,
            NETWORK,
            AuditStatus::Fail,
            format!(
                "RPC endpoint has no TLS and listens on {addr}: requests and replies cross \
                 the network unencrypted"
            ),
        ),
        Endpoint::Invalid { .. } | Endpoint::Unknown { .. } => result(
            name,
            NETWORK,
            AuditStatus::Warn,
            "RPC endpoint has no TLS, and its listen address could not be determined".to_string(),
        ),
    }
}

/// RPC methods are gated by a rate limiter and nothing else. There is no
/// credential to look for, so this never passes; how bad the absence is
/// depends on who can reach the endpoint.
pub(super) fn check_authentication(endpoint: &Endpoint) -> AuditResult {
    let name = "Authentication";
    match endpoint {
        Endpoint::Bound { addr, .. } if is_loopback(addr.ip()) => result(
            name,
            AUTHENTICATION,
            AuditStatus::Warn,
            format!(
                "RPC methods do not authenticate callers; any local process can call them \
                 (the endpoint listens on {addr})"
            ),
        ),
        Endpoint::Bound { addr, .. } => result(
            name,
            AUTHENTICATION,
            AuditStatus::Fail,
            format!(
                "RPC methods do not authenticate callers and the endpoint listens on {addr}: \
                 any host that can reach it can call them"
            ),
        ),
        Endpoint::Invalid { .. } | Endpoint::Unknown { .. } => result(
            name,
            AUTHENTICATION,
            AuditStatus::Warn,
            "RPC methods do not authenticate callers, and the endpoint's listen address \
             could not be determined"
                .to_string(),
        ),
    }
}

/// `config` is the rate limit the agent runs its endpoint with. The limiter
/// the RPC handlers consult is shared by all callers: it caps total load, it
/// does not hold back one caller in favour of another.
pub(super) fn check_rate_limiting(config: &RateLimitConfig) -> AuditResult {
    let name = "Rate limiting";
    if !config.enabled {
        return result(
            name,
            NETWORK,
            AuditStatus::Fail,
            "RPC rate limiting is disabled".to_string(),
        );
    }
    if config.max_requests_per_second == 0 || config.burst_size == 0 {
        return result(
            name,
            NETWORK,
            AuditStatus::Fail,
            format!(
                "RPC rate limit is not usable: {} requests/s, burst {}",
                config.max_requests_per_second, config.burst_size
            ),
        );
    }
    result(
        name,
        NETWORK,
        AuditStatus::Pass,
        format!(
            "RPC endpoint allows {} requests/s (burst {}) in total; the limit is shared by \
             all callers, not applied per client",
            config.max_requests_per_second, config.burst_size
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::listen::rpc_rate_limit;

    const KIT: &str = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "hello"
  version: "0.1.0"
  authors:
    - did: "did:web:example.com"
  created: "2026-04-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "say hello"
roles:
  - id: agent
    role_type: operator
    agent_provisioning: {}
    skills: []
    mcp_tools: []
    handoffs: []
coordination:
  topology: hub-spoke
  protocol: a2a-jsonrpc-2.0
  routing:
    strategy: static
constraints:
  global_budget:
    max_wallclock_seconds: 60
    max_total_tokens: 8000
    max_cost_usd: 0.01
  data_classifications: ["public"]
  network:
    egress_allowed: false
    egress_allowlist: []
completion:
  rules: ["done"]
  on_failure: abort
  max_retries: 0
provenance:
  signatures:
    - signer_did: "did:web:example.com"
      algorithm: ed25519
      signature: "AAA"
"#;

    /// A directory whose discovered kit has `listen` as its runtime.listen,
    /// or no runtime block at all.
    fn dir_with_kit(listen: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let kit = match listen {
            Some(listen) => KIT.replacen(
                "kit:",
                &format!("runtime:\n  listen: \"{listen}\"\nkit:"),
                1,
            ),
            None => KIT.to_string(),
        };
        std::fs::write(dir.path().join("agent.swarmkit.yaml"), kit).unwrap();
        dir
    }

    fn endpoint_checks(endpoint: &Endpoint) -> [AuditResult; 3] {
        [
            check_bind(endpoint),
            check_transport(endpoint),
            check_authentication(endpoint),
        ]
    }

    /// Regression: with no kit the audit reported a loopback endpoint and
    /// passed, while the agent it describes listens on every interface.
    #[test]
    fn no_kit_is_audited_as_the_built_in_default_on_every_interface() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = effective_endpoint(dir.path());

        for check in endpoint_checks(&endpoint) {
            assert_eq!(check.status, AuditStatus::Fail, "{}", check.message);
            assert!(check.message.contains("0.0.0.0:0"), "{}", check.message);
            assert!(
                !check.message.contains("this machine only"),
                "{}",
                check.message
            );
        }
        let bind = check_bind(&endpoint);
        assert!(
            bind.message.contains("built-in default, no kit found"),
            "{}",
            bind.message
        );
        assert!(
            bind.message.contains("reachable from the network"),
            "{}",
            bind.message
        );
        assert!(bind.message.contains("--trust"), "{}", bind.message);
    }

    #[test]
    fn a_kit_without_runtime_listen_is_audited_as_the_default() {
        let dir = dir_with_kit(None);
        let bind = check_bind(&effective_endpoint(dir.path()));

        assert_eq!(bind.status, AuditStatus::Fail);
        assert!(bind.message.contains("0.0.0.0:0"), "{}", bind.message);
        assert!(
            bind.message.contains("sets no runtime.listen"),
            "{}",
            bind.message
        );
    }

    /// Regression: the bind check passed with "Default bind is
    /// localhost-only" whatever the kit said.
    #[test]
    fn a_kit_listening_on_the_network_fails_the_endpoint_checks() {
        for listen in ["0.0.0.0:8342", "[::]:8342", "10.0.0.140:8342"] {
            let dir = dir_with_kit(Some(listen));
            for check in endpoint_checks(&effective_endpoint(dir.path())) {
                assert_eq!(
                    check.status,
                    AuditStatus::Fail,
                    "{} must fail for {listen}: {}",
                    check.name,
                    check.message
                );
                assert!(check.message.contains(listen), "{}", check.message);
            }
        }
    }

    #[test]
    fn a_kit_listening_on_loopback_passes_the_bind_check() {
        for listen in ["127.0.0.1:8342", "[::1]:8342"] {
            let dir = dir_with_kit(Some(listen));
            let bind = check_bind(&effective_endpoint(dir.path()));

            assert_eq!(bind.status, AuditStatus::Pass, "{}", bind.message);
            assert!(bind.message.contains(listen), "{}", bind.message);
            assert!(bind.message.contains("runtime.listen"), "{}", bind.message);
        }
    }

    #[test]
    fn an_unparseable_listen_address_fails_the_bind_check() {
        let dir = dir_with_kit(Some("localhost:8342"));
        let bind = check_bind(&effective_endpoint(dir.path()));

        assert_eq!(bind.status, AuditStatus::Fail);
        assert!(bind.message.contains("will not start"), "{}", bind.message);
    }

    #[test]
    fn a_kit_that_cannot_be_loaded_passes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("agent.swarmkit.yaml"), "not: [a, kit").unwrap();

        for check in endpoint_checks(&effective_endpoint(dir.path())) {
            assert_eq!(check.status, AuditStatus::Warn, "{}", check.message);
        }
    }

    /// Regression: authentication passed when `JWT_SECRET` was set, a
    /// variable the RPC endpoint never reads.
    #[test]
    fn authentication_never_passes() {
        let dirs = [
            tempfile::tempdir().unwrap(),
            dir_with_kit(None),
            dir_with_kit(Some("127.0.0.1:8342")),
            dir_with_kit(Some("0.0.0.0:8342")),
            dir_with_kit(Some("nonsense")),
        ];
        for dir in &dirs {
            let auth = check_authentication(&effective_endpoint(dir.path()));
            assert_ne!(auth.status, AuditStatus::Pass, "{}", auth.message);
            assert!(
                auth.message.contains("do not authenticate"),
                "{}",
                auth.message
            );
        }
    }

    /// Regression: rate limiting passed without looking at any
    /// configuration.
    #[test]
    fn rate_limiting_reports_the_limit_the_agent_runs_with() {
        let config = rpc_rate_limit();
        let check = check_rate_limiting(&config);

        assert_eq!(check.status, AuditStatus::Pass, "{}", check.message);
        assert!(
            check
                .message
                .contains(&format!("{} requests/s", config.max_requests_per_second)),
            "{}",
            check.message
        );
        assert!(check.message.contains("not applied per client"));
    }

    #[test]
    fn a_disabled_or_empty_rate_limit_fails() {
        let disabled = RateLimitConfig {
            enabled: false,
            ..RateLimitConfig::default()
        };
        assert_eq!(check_rate_limiting(&disabled).status, AuditStatus::Fail);

        let empty = RateLimitConfig {
            max_requests_per_second: 0,
            ..RateLimitConfig::default()
        };
        assert_eq!(check_rate_limiting(&empty).status, AuditStatus::Fail);
    }
}
