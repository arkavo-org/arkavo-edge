//! Errors of the kit configuration methods (`agent.config.get`,
//! `agent.config.update`, `agent.config.restore`).
//!
//! An agent serves these only while it listens on a loopback address. One
//! that listens anywhere else leaves them unregistered, because its endpoint
//! does not authenticate callers, and answers a call with JSON-RPC's
//! "Method not found". That answer reads like a version mismatch or a typo;
//! the user needs to be told it is a decision and where the kit can be
//! changed instead.

use jsonrpsee::core::client::Error as RpcError;
use jsonrpsee::types::error::METHOD_NOT_FOUND_CODE;

/// The agent at `endpoint` does not serve the kit configuration method
/// `method`.
#[derive(Debug, thiserror::Error)]
#[error(
    "agent '{agent_id}' at {endpoint} does not serve {method}. An agent serves its kit \
     configuration only when it listens on a loopback address, because the endpoint does not \
     authenticate callers. Edit the kit file on the agent's machine, or have the agent listen \
     on loopback (runtime.listen: \"127.0.0.1:0\") and restart it"
)]
pub struct KitConfigNotServed {
    pub agent_id: String,
    pub endpoint: String,
    pub method: &'static str,
}

/// The error to report for a failed call of `method`: the explanation when
/// the agent withholds the method, the RPC error itself otherwise.
pub(super) fn call_error(
    agent_id: &str,
    endpoint: &str,
    method: &'static str,
    error: RpcError,
) -> Box<dyn std::error::Error + Send + Sync> {
    match error {
        RpcError::Call(call) if call.code() == METHOD_NOT_FOUND_CODE => {
            Box::new(KitConfigNotServed {
                agent_id: agent_id.to_string(),
                endpoint: endpoint.to_string(),
                method,
            })
        }
        other => Box::new(other),
    }
}

#[cfg(test)]
mod tests {
    use super::super::AgentConnection;
    use super::*;
    use jsonrpsee::server::{RpcModule, Server, ServerHandle};
    use jsonrpsee::types::ErrorObjectOwned;
    use tokio::sync::mpsc;

    /// An endpoint that serves what an agent bound to a non-loopback address
    /// serves of the kit configuration methods: `agent.config.validate` and
    /// nothing else. It listens on loopback itself; what it leaves
    /// unregistered is what matters to the client.
    async fn agent_withholding_kit_configuration() -> (ServerHandle, String) {
        let mut module = RpcModule::new(());
        module
            .register_method(
                "agent.config.validate",
                |_, (), _| serde_json::json!({ "valid": true, "errors": [], "warnings": [] }),
            )
            .unwrap();
        let server = Server::builder().build("127.0.0.1:0").await.unwrap();
        let endpoint = server.local_addr().unwrap().to_string();
        (server.start(module), endpoint)
    }

    async fn connected(endpoint: &str) -> AgentConnection {
        let (telemetry_tx, _telemetry_rx) = mpsc::channel(4);
        let connection =
            AgentConnection::new("planner".to_string(), endpoint.to_string(), telemetry_tx);
        connection.connect().await.expect("connects");
        connection
    }

    fn assert_explains(
        error: &(dyn std::error::Error + Send + Sync),
        method: &str,
        endpoint: &str,
    ) {
        let message = error.to_string();
        assert!(message.contains(method), "{message}");
        assert!(message.contains(endpoint), "{message}");
        assert!(message.contains("loopback"), "{message}");
        assert!(message.contains("runtime.listen"), "{message}");
        assert!(!message.contains("Method not found"), "{message}");
    }

    /// Regression: an agent that withholds its kit configuration answered
    /// "Method not found", and that was all the user was shown.
    #[tokio::test]
    async fn a_withheld_method_is_explained_to_the_user() {
        let (server, endpoint) = agent_withholding_kit_configuration().await;
        let connection = connected(&endpoint).await;

        let error = connection.get_config(false).await.unwrap_err();
        assert_explains(error.as_ref(), "agent.config.get", &endpoint);

        let error = connection
            .update_config("kit: {}".to_string(), None, true)
            .await
            .unwrap_err();
        assert_explains(error.as_ref(), "agent.config.update", &endpoint);

        let error = connection
            .restore_config("agent.swarmkit.yaml.bak".to_string())
            .await
            .unwrap_err();
        assert_explains(error.as_ref(), "agent.config.restore", &endpoint);

        server.stop().unwrap();
    }

    #[test]
    fn other_failures_are_reported_as_they_are() {
        let invalid_params = ErrorObjectOwned::owned(-32602, "kit.id is required", None::<()>);
        let error = call_error(
            "planner",
            "127.0.0.1:8431",
            "agent.config.update",
            RpcError::Call(invalid_params),
        );
        assert!(error.to_string().contains("kit.id is required"), "{error}");
        assert!(!error.to_string().contains("loopback"), "{error}");

        let error = call_error(
            "planner",
            "127.0.0.1:8431",
            "agent.config.get",
            RpcError::RequestTimeout,
        );
        assert!(!error.to_string().contains("loopback"), "{error}");
    }
}
