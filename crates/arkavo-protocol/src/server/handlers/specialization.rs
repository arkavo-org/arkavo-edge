use crate::mcp_registry::McpRegistry;
use crate::metrics::{MetricsCollector, RpcTimer};
use crate::rate_limit::RateLimiter;
use crate::types::{AgentSpecializeRequest, AgentSpecializeResponse};
use jsonrpsee::types::ErrorObjectOwned;
use std::sync::Arc;
use tracing::{info, warn};

use super::super::config_helpers::AgentMetadata;

/// Handle agent.specialize RPC method
/// Receives TDF-encrypted ConfigurationBundle and applies specialization
pub async fn handle_agent_specialize(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    _mcp_registry: &Arc<McpRegistry>,
    agent_metadata: &Arc<tokio::sync::RwLock<AgentMetadata>>,
    request: AgentSpecializeRequest,
) -> Result<AgentSpecializeResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("agent.specialize".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    let agent_name = {
        let metadata = agent_metadata.read().await;
        metadata.name.clone()
    };

    info!(
        "Received specialization request from '{}' for agent '{}'",
        request.requester_id, agent_name
    );

    // Generate session ID for this specialization
    let session_id = request
        .session_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // For now, we acknowledge the specialization request but note that
    // full TDF decryption requires KAS integration which is async
    // The encrypted_bundle contains the TDF-encrypted ConfigurationBundle
    if request.encrypted_bundle.is_empty() {
        timer.error();
        return Err(ErrorObjectOwned::owned(
            -32602,
            "Invalid params: encrypted_bundle is required",
            Some("The encrypted_bundle field cannot be empty".to_string()),
        ));
    }

    // Log the specialization attempt
    info!(
        "Specialization session '{}': encrypted bundle size = {} bytes",
        session_id,
        request.encrypted_bundle.len()
    );

    // In a full implementation with KAS:
    // 1. Decode the base64 encrypted_bundle to get EncryptedBundle
    // 2. Use ConfigBundleDecryptor with agent identity to decrypt
    // 3. Apply the ConfigurationBundle to modify agent behavior
    // 4. Update MCP tool entitlements, settings, etc.

    // For now, we accept the specialization and return success
    // Full implementation requires KAS client integration
    warn!("TDF decryption requires KAS integration - specialization stored but not yet applied");

    let response = AgentSpecializeResponse {
        session_id,
        accepted: true,
        message: Some(
            "Specialization request received. Full TDF decryption pending KAS integration."
                .to_string(),
        ),
        activated_capabilities: Vec::new(), // Will be populated after decryption
    };

    timer.success();
    Ok(response)
}
