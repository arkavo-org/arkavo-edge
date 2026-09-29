//! RPC methods that are served only when the endpoint is bound to loopback.
//!
//! The server is started through jsonrpsee's `Server::start`, which accepts
//! connections itself and hands a method handler the request alone: the
//! peer's address never reaches it. A handler therefore cannot tell a local
//! caller from a remote one, and the only place the distinction can be
//! enforced is the bind address. Bound to loopback, every caller is on this
//! machine. Bound to anything else, the methods below are not registered at
//! all, so no caller, local or remote, can reach them on that endpoint.

use std::net::IpAddr;

use arkavo_protocol::error::{A2aError, Result};
use jsonrpsee::RpcModule;

/// Methods that return the kit file or replace it on disk. The kit holds the
/// agent's instructions, tool grants and spend policy, and a replaced kit is
/// hot-reloaded into the running agent.
pub(super) const LOCAL_ONLY_METHODS: [&str; 3] = [
    "agent.config.get",
    "agent.config.update",
    "agent.config.restore",
];

/// Whether only processes on this machine can connect to `ip`.
///
/// An IPv4-mapped IPv6 address is judged by the IPv4 address it carries.
pub(super) fn is_loopback(ip: IpAddr) -> bool {
    ip.to_canonical().is_loopback()
}

/// Remove [`LOCAL_ONLY_METHODS`] from `module` unless `bound` is loopback.
/// Returns the names that were removed.
///
/// Every listed method must be registered on `module`, whatever the bind
/// address. A name that matches nothing means a method was renamed without
/// this list following it, and the renamed method would be served to the
/// network; refusing to start is the only safe answer to that.
pub(super) fn restrict_to_bound_address<Context: Send + Sync + 'static>(
    module: &mut RpcModule<Context>,
    bound: IpAddr,
) -> Result<Vec<&'static str>> {
    for name in LOCAL_ONLY_METHODS {
        if module.method(name).is_none() {
            return Err(A2aError::Internal(format!(
                "local-only RPC method {name:?} is not registered; refusing to start \
                 with an unknown method surface"
            )));
        }
    }

    if is_loopback(bound) {
        return Ok(Vec::new());
    }

    for name in LOCAL_ONLY_METHODS {
        module.remove_method(name);
    }
    Ok(LOCAL_ONLY_METHODS.to_vec())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use jsonrpsee::core::server::MethodsError;
    use jsonrpsee::types::ErrorCode;

    fn module_with(names: &[&'static str]) -> RpcModule<()> {
        let mut module = RpcModule::new(());
        for name in names {
            module
                .register_method(name, |_, (), _| "served")
                .expect("method names in a test module are unique");
        }
        module
    }

    fn full_module() -> RpcModule<()> {
        let mut names = LOCAL_ONLY_METHODS.to_vec();
        names.push("health");
        module_with(&names)
    }

    async fn call(module: &RpcModule<()>, method: &str) -> std::result::Result<String, i32> {
        match module.call::<[(); 0], String>(method, []).await {
            Ok(reply) => Ok(reply),
            Err(MethodsError::JsonRpc(e)) => Err(e.code()),
            Err(other) => panic!("unexpected failure calling {method}: {other}"),
        }
    }

    #[tokio::test]
    async fn a_network_bind_does_not_serve_the_kit_methods() {
        for bound in [
            "0.0.0.0",
            "::",
            "10.0.0.140",
            "fe80::1",
            "::ffff:10.0.0.140",
        ] {
            let mut module = full_module();
            let removed = restrict_to_bound_address(&mut module, bound.parse().unwrap()).unwrap();

            assert_eq!(removed, LOCAL_ONLY_METHODS.to_vec(), "bound to {bound}");
            for name in LOCAL_ONLY_METHODS {
                assert_eq!(
                    call(&module, name).await,
                    Err(ErrorCode::MethodNotFound.code()),
                    "{name} must not be served on {bound}"
                );
            }
            assert_eq!(call(&module, "health").await, Ok("served".to_string()));
        }
    }

    #[tokio::test]
    async fn a_loopback_bind_serves_the_kit_methods() {
        for bound in ["127.0.0.1", "127.8.9.10", "::1", "::ffff:127.0.0.1"] {
            let mut module = full_module();
            let removed = restrict_to_bound_address(&mut module, bound.parse().unwrap()).unwrap();

            assert!(removed.is_empty(), "bound to {bound}");
            for name in LOCAL_ONLY_METHODS {
                assert_eq!(call(&module, name).await, Ok("served".to_string()));
            }
        }
    }

    /// The list above is only as good as its match with the names the RPC
    /// trait registers, so this goes through the real implementation.
    #[tokio::test]
    async fn the_servers_own_methods_are_withheld_from_a_network_bind() {
        use crate::server::A2aRpcServer;

        let metadata = std::sync::Arc::new(tokio::sync::RwLock::new(
            crate::server::config_helpers::AgentMetadata::default(),
        ));
        let mut module = crate::server::test_support::rpc_impl(metadata)
            .await
            .into_rpc();
        for name in LOCAL_ONLY_METHODS {
            assert!(module.method(name).is_some(), "{name} is not an RPC method");
        }

        restrict_to_bound_address(&mut module, "0.0.0.0".parse().unwrap()).unwrap();

        // Whatever is left under `agent.config.` neither reads nor writes
        // the kit file: a method added there later has to be classified.
        let mut config_methods: Vec<&str> = module
            .method_names()
            .filter(|name| name.starts_with("agent.config."))
            .collect();
        config_methods.sort_unstable();
        assert_eq!(config_methods, ["agent.config.validate"]);
        assert!(module.method("health").is_some());
        assert!(module.method("message/send").is_some());
    }

    #[test]
    fn a_listed_method_that_is_not_registered_stops_startup() {
        for bound in ["127.0.0.1", "0.0.0.0"] {
            let mut module = module_with(&["agent.config.get", "agent.config.restore", "health"]);
            let err = restrict_to_bound_address(&mut module, bound.parse().unwrap())
                .expect_err("a missing local-only method must be an error");
            assert!(err.to_string().contains("agent.config.update"), "{err}");
        }
    }
}
