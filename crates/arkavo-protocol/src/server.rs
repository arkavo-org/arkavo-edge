use crate::config::ServerConfig;
use crate::error::{A2aError, Result};
use crate::metrics::{MetricsCollector, RpcTimer};
use crate::mcp_registry::McpRegistry;
use crate::openrpc;
use crate::rate_limit::RateLimiter;
#[cfg(feature = "stub_handlers")]
use crate::types::PromiseStatus;
use crate::types::{
    AgentDiscoverFilter, DiscoveredAgent, PromiseCapability, PromiseDeclareResponse,
    PromiseResponse,
};
use async_trait::async_trait;
use jsonrpsee::server::{ServerBuilder, ServerHandle};
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;

#[rpc(server)]
pub trait A2aRpc {
    #[method(name = "promise_request")]
    async fn promise_request(
        &self,
        agent_id: String,
        promise_type: String,
        payload: Option<serde_json::Value>,
    ) -> RpcResult<PromiseResponse>;

    #[method(name = "promise_declare")]
    async fn promise_declare(
        &self,
        agent_id: String,
        promises: Vec<PromiseCapability>,
    ) -> RpcResult<PromiseDeclareResponse>;

    #[method(name = "agent_discover")]
    async fn agent_discover(
        &self,
        filter: Option<AgentDiscoverFilter>,
    ) -> RpcResult<Vec<DiscoveredAgent>>;

    #[method(name = "rpc.discover")]
    async fn rpc_discover(&self) -> RpcResult<serde_json::Value>;
}

pub struct A2aRpcImpl {
    rate_limiter: Arc<RateLimiter>,
    metrics: Arc<MetricsCollector>,
    mcp_registry: Arc<McpRegistry>,
    agent_metadata: Arc<tokio::sync::RwLock<AgentMetadata>>,
}

#[derive(Default)]
struct AgentMetadata {
    name: String,
    purpose: String,
    model: String,
    endpoint: String,
}

#[async_trait]
impl A2aRpcServer for A2aRpcImpl {
    async fn promise_request(
        &self,
        _agent_id: String,
        _promise_type: String,
        _payload: Option<serde_json::Value>,
    ) -> RpcResult<PromiseResponse> {
        let timer = RpcTimer::new("promise_request".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        #[cfg(feature = "stub_handlers")]
        {
            let response = PromiseResponse {
                promise_id: uuid::Uuid::new_v4(),
                status: PromiseStatus::Pending,
                data: _payload,
            };
            timer.success();
            Ok(response)
        }

        #[cfg(not(feature = "stub_handlers"))]
        {
            timer.error();
            Err(ErrorObjectOwned::owned(
                -32601,
                "Method not yet implemented",
                Some("promise_request is not yet implemented".to_string()),
            ))
        }
    }

    async fn promise_declare(
        &self,
        _agent_id: String,
        _promises: Vec<PromiseCapability>,
    ) -> RpcResult<PromiseDeclareResponse> {
        let timer = RpcTimer::new("promise_declare".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        #[cfg(feature = "stub_handlers")]
        {
            let response = PromiseDeclareResponse {
                acknowledged: true,
                timestamp: chrono::Utc::now(),
            };
            timer.success();
            Ok(response)
        }

        #[cfg(not(feature = "stub_handlers"))]
        {
            timer.error();
            Err(ErrorObjectOwned::owned(
                -32601,
                "Method not yet implemented",
                Some("promise_declare is not yet implemented".to_string()),
            ))
        }
    }

    async fn agent_discover(
        &self,
        _filter: Option<AgentDiscoverFilter>,
    ) -> RpcResult<Vec<DiscoveredAgent>> {
        let timer = RpcTimer::new("agent_discover".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        // Get agent metadata
        let metadata = self.agent_metadata.read().await;
        
        // Get MCP tools and server status
        let mcp_tools = match self.mcp_registry.list_all_tools().await {
            Ok(tools) => tools.into_iter().map(|t| t.name).collect::<Vec<String>>(),
            Err(_) => Vec::new(),
        };
        
        let mcp_servers = self.mcp_registry.get_server_status().await;
        
        // Build metadata with MCP information
        let metadata_json = serde_json::json!({
            "name": metadata.name,
            "purpose": metadata.purpose,
            "model": metadata.model,
            "mcp_tools": mcp_tools,
            "mcp_servers": mcp_servers,
        });

        let agent = DiscoveredAgent {
            agent_id: uuid::Uuid::new_v4(), // Generate a unique ID for the agent
            endpoint: metadata.endpoint.clone(),
            promises: Some(vec![]), // TODO: Populate with actual promise types
            metadata: Some(metadata_json),
        };

        timer.success();
        Ok(vec![agent])
    }

    async fn rpc_discover(&self) -> RpcResult<serde_json::Value> {
        let timer = RpcTimer::new("rpc.discover".to_string(), self.metrics.clone());
        let schema = openrpc::generate_openrpc_schema();

        match serde_json::to_value(schema) {
            Ok(value) => {
                timer.success();
                Ok(value)
            }
            Err(e) => {
                timer.error();
                Err(ErrorObjectOwned::owned(
                    -32603,
                    "Failed to serialize OpenRPC schema",
                    Some(e.to_string()),
                ))
            }
        }
    }
}

pub struct A2aServer {
    config: ServerConfig,
    mcp_registry: Arc<McpRegistry>,
    agent_metadata: Arc<tokio::sync::RwLock<AgentMetadata>>,
}

impl A2aServer {
    pub fn new(config: ServerConfig) -> Self {
        Self { 
            config,
            mcp_registry: Arc::new(McpRegistry::new()),
            agent_metadata: Arc::new(tokio::sync::RwLock::new(AgentMetadata::default())),
        }
    }
    
    pub fn mcp_registry(&self) -> Arc<McpRegistry> {
        self.mcp_registry.clone()
    }
    
    pub async fn set_agent_metadata(&self, name: String, purpose: String, model: String) {
        let mut metadata = self.agent_metadata.write().await;
        metadata.name = name;
        metadata.purpose = purpose;
        metadata.model = model;
        metadata.endpoint = format!("http://{}:{}", self.config.bind_address, self.config.port);
    }

    pub async fn start(&self) -> Result<ServerHandle> {
        let addr: SocketAddr = format!("{}:{}", self.config.bind_address, self.config.port)
            .parse()
            .map_err(|e| A2aError::InvalidEndpoint(format!("Invalid bind address: {e}")))?;

        info!("Starting A2A server on {}", addr);

        let server = ServerBuilder::default()
            .max_connections(self.config.max_connections as u32)
            .build(addr)
            .await
            .map_err(|e| A2aError::Transport(format!("Failed to build server: {e}")))?;

        let rate_limiter = Arc::new(RateLimiter::new(self.config.rate_limit.clone()));
        let metrics = Arc::new(MetricsCollector::new(true)); // TODO: Make configurable
        let rpc_impl = A2aRpcImpl {
            rate_limiter,
            metrics,
            mcp_registry: self.mcp_registry.clone(),
            agent_metadata: self.agent_metadata.clone(),
        };
        let handle = server.start(rpc_impl.into_rpc());

        info!("A2A server started successfully on {}", addr);
        info!("OpenRPC schema available via JSON-RPC method: rpc.discover");

        Ok(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_server_creation() {
        let config = ServerConfig::default();
        let _server = A2aServer::new(config);
        // Server creation should succeed
    }

    #[tokio::test]
    async fn test_promise_request_handler() {
        let mut config = crate::rate_limit::RateLimitConfig::default();
        config.max_requests_per_second = 100;
        let rate_limiter = Arc::new(crate::rate_limit::RateLimiter::new(config));
        let metrics = Arc::new(MetricsCollector::new(false));
        let impl_instance = A2aRpcImpl {
            rate_limiter,
            metrics,
            mcp_registry: Arc::new(McpRegistry::new()),
            agent_metadata: Arc::new(tokio::sync::RwLock::new(AgentMetadata::default())),
        };
        let result = impl_instance
            .promise_request(
                "test-agent".to_string(),
                "data_access".to_string(),
                Some(serde_json::json!({"key": "value"})),
            )
            .await;

        #[cfg(feature = "stub_handlers")]
        {
            let result = result.unwrap();
            assert!(result.promise_id.to_string().len() > 0);
            assert!(matches!(result.status, PromiseStatus::Pending));
        }

        #[cfg(not(feature = "stub_handlers"))]
        {
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.code(), -32601);
            assert!(err.message().contains("not yet implemented"));
        }
    }

    #[tokio::test]
    async fn test_promise_declare_handler() {
        let mut config = crate::rate_limit::RateLimitConfig::default();
        config.max_requests_per_second = 100;
        let rate_limiter = Arc::new(crate::rate_limit::RateLimiter::new(config));
        let metrics = Arc::new(MetricsCollector::new(false));
        let impl_instance = A2aRpcImpl {
            rate_limiter,
            metrics,
            mcp_registry: Arc::new(McpRegistry::new()),
            agent_metadata: Arc::new(tokio::sync::RwLock::new(AgentMetadata::default())),
        };
        let result = impl_instance
            .promise_declare(
                "test-agent".to_string(),
                vec![PromiseCapability {
                    promise_type: "compute".to_string(),
                    constraints: None,
                    metadata: None,
                }],
            )
            .await;

        #[cfg(feature = "stub_handlers")]
        {
            let result = result.unwrap();
            assert!(result.acknowledged);
            assert!(result.timestamp <= chrono::Utc::now());
        }

        #[cfg(not(feature = "stub_handlers"))]
        {
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.code(), -32601);
            assert!(err.message().contains("not yet implemented"));
        }
    }

    #[tokio::test]
    async fn test_rpc_discover() {
        let mut config = crate::rate_limit::RateLimitConfig::default();
        config.max_requests_per_second = 100;
        let rate_limiter = Arc::new(crate::rate_limit::RateLimiter::new(config));
        let metrics = Arc::new(MetricsCollector::new(false));
        let impl_instance = A2aRpcImpl {
            rate_limiter,
            metrics,
            mcp_registry: Arc::new(McpRegistry::new()),
            agent_metadata: Arc::new(tokio::sync::RwLock::new(AgentMetadata::default())),
        };
        let result = impl_instance.rpc_discover().await.unwrap();

        assert_eq!(result.get("openrpc").unwrap(), "1.2.6");
        assert!(result.get("info").is_some());
        assert!(result.get("methods").is_some());

        let methods = result.get("methods").unwrap().as_array().unwrap();
        assert!(methods.len() >= 3);

        let method_names: Vec<&str> = methods
            .iter()
            .filter_map(|m| m.get("name").and_then(|n| n.as_str()))
            .collect();

        assert!(method_names.contains(&"promise_request"));
        assert!(method_names.contains(&"promise_declare"));
        assert!(method_names.contains(&"agent_discover"));
    }

    #[tokio::test]
    async fn test_agent_discover_handler() {
        let mut config = crate::rate_limit::RateLimitConfig::default();
        config.max_requests_per_second = 100;
        let rate_limiter = Arc::new(crate::rate_limit::RateLimiter::new(config));
        let metrics = Arc::new(MetricsCollector::new(false));
        let impl_instance = A2aRpcImpl {
            rate_limiter,
            metrics,
            mcp_registry: Arc::new(McpRegistry::new()),
            agent_metadata: Arc::new(tokio::sync::RwLock::new(AgentMetadata::default())),
        };
        let result = impl_instance
            .agent_discover(Some(AgentDiscoverFilter {
                promise_types: Some(vec!["test".to_string()]),
                tags: None,
            }))
            .await;

        #[cfg(feature = "stub_handlers")]
        {
            let result = result.unwrap();
            assert!(result.is_empty());
        }

        #[cfg(not(feature = "stub_handlers"))]
        {
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.code(), -32601);
            assert!(err.message().contains("not yet implemented"));
        }
    }

    #[tokio::test]
    async fn test_rate_limiting() {
        let mut config = crate::rate_limit::RateLimitConfig::default();
        config.max_requests_per_second = 1;
        config.burst_size = 1;
        let rate_limiter = Arc::new(crate::rate_limit::RateLimiter::new(config));
        let metrics = Arc::new(MetricsCollector::new(false));
        let impl_instance = A2aRpcImpl {
            rate_limiter,
            metrics,
            mcp_registry: Arc::new(McpRegistry::new()),
            agent_metadata: Arc::new(tokio::sync::RwLock::new(AgentMetadata::default())),
        };

        // First request should succeed
        let result1 = impl_instance.agent_discover(None).await;

        #[cfg(feature = "stub_handlers")]
        assert!(result1.is_ok());
        #[cfg(not(feature = "stub_handlers"))]
        assert_eq!(result1.unwrap_err().code(), -32601);

        // Second request should be rate limited
        let result2 = impl_instance.agent_discover(None).await;

        assert!(result2.is_err());
        let err = result2.unwrap_err();
        assert_eq!(err.code(), -32001);
        assert!(err.message().contains("Rate limit"));
    }
}
