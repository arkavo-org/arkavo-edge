//! Integration example for config transport with A2A RPC server
//!
//! This module provides example code showing how to integrate the secure
//! configuration transport with the A2A RPC server implementation.

use super::ConfigTransportHandler;
use crate::Result;
use crate::types::{AgentConfigGetRequest, AgentConfigGetResponse};

/// Example integration of ConfigTransportHandler into A2aRpcImpl
///
/// This shows how to modify the agent_config_get method to support
/// secure configuration bundle distribution.
///
/// # Example
///
/// ```rust,ignore
/// // In A2aRpcImpl struct, add:
/// pub struct A2aRpcImpl {
///     // ... existing fields ...
///     config_transport: Option<Arc<ConfigTransportHandler>>,
/// }
///
/// // In agent_config_get implementation:
/// async fn agent_config_get(
///     &self,
///     request: AgentConfigGetRequest,
/// ) -> RpcResult<AgentConfigGetResponse> {
///     let timer = RpcTimer::new("agent_config_get".to_string(), self.metrics.clone());
///
///     // Check rate limit
///     if let Err(e) = self.rate_limiter.check_rate_limit() {
///         self.metrics.record_rate_limit_blocked(None);
///         timer.error();
///         return Err(e);
///     }
///
///     // Try secure config transport first
///     if let Some(config_transport) = &self.config_transport {
///         if let Ok(Some(response)) = config_transport.handle_config_request(&request).await {
///             timer.success();
///             return Ok(response);
///         }
///     }
///
///     // Fall back to standard AGENTS.md config
///     // ... existing AGENTS.md reading logic ...
/// }
/// ```
pub struct ConfigTransportIntegration;

impl ConfigTransportIntegration {
    /// Create a new config transport handler for integration
    pub fn create_handler(kas_url: String) -> ConfigTransportHandler {
        ConfigTransportHandler::new(kas_url)
    }

    /// Example: Handle agent registration with public key
    ///
    /// This would be called when an agent first connects and provides its public key.
    pub async fn handle_agent_registration(
        handler: &ConfigTransportHandler,
        agent_id: String,
        public_key: Vec<u8>,
    ) -> Result<()> {
        handler.register_agent_key(agent_id, public_key).await;
        Ok(())
    }

    /// Example: Enhanced agent_config_get with secure transport
    ///
    /// This shows the complete flow of handling a config request with fallback.
    pub async fn enhanced_agent_config_get(
        handler: Option<&ConfigTransportHandler>,
        request: AgentConfigGetRequest,
        fallback_fn: impl FnOnce(AgentConfigGetRequest) -> Result<AgentConfigGetResponse>,
    ) -> Result<AgentConfigGetResponse> {
        // Try secure config transport first
        if let Some(handler) = handler
            && let Ok(Some(response)) = handler.handle_config_request(&request).await
        {
            return Ok(response);
        }

        // Fall back to standard config
        fallback_fn(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_handler() {
        let handler =
            ConfigTransportIntegration::create_handler("https://kas.example.com".to_string());
        assert_eq!(handler.kas_url(), "https://kas.example.com");
    }

    #[tokio::test]
    async fn test_agent_registration() {
        let handler =
            ConfigTransportIntegration::create_handler("https://kas.example.com".to_string());

        let result = ConfigTransportIntegration::handle_agent_registration(
            &handler,
            "test-agent-001".to_string(),
            vec![1, 2, 3, 4, 5],
        )
        .await;

        assert!(result.is_ok());

        let key = handler
            .key_registry()
            .get_public_key("test-agent-001")
            .await;
        assert!(key.is_some());
    }
}
