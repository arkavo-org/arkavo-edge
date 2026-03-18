//! Iroh node lifecycle management.
//!
//! Manages an embedded Iroh endpoint with blob protocol support.

use iroh::Endpoint;
use iroh::protocol::Router;
use iroh_base::EndpointId;
use iroh_blobs::BlobsProtocol;
use iroh_blobs::store::mem::MemStore;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::error::IrohError;

/// Configuration for the Iroh node.
#[derive(Debug, Clone)]
pub struct IrohNodeConfig {
    /// Enable relay mode for NAT traversal (default: true)
    pub relay_mode: bool,
}

impl Default for IrohNodeConfig {
    fn default() -> Self {
        Self { relay_mode: true }
    }
}

impl IrohNodeConfig {
    /// Create a new config with relay mode disabled (direct connections only).
    #[must_use]
    pub fn direct_only() -> Self {
        Self { relay_mode: false }
    }
}

/// State of a running Iroh node.
struct RunningNode {
    endpoint: Endpoint,
    store: MemStore,
    router: Router,
}

/// Embedded Iroh node for blob storage and transfer.
///
/// Manages the lifecycle of:
/// - Iroh endpoint (P2P connections)
/// - Blob store (in-memory)
/// - Protocol router (handles blob requests)
pub struct IrohNode {
    #[allow(dead_code)] // Reserved for relay configuration
    config: IrohNodeConfig,
    state: Arc<RwLock<Option<RunningNode>>>,
}

impl IrohNode {
    /// Create a new Iroh node (not yet started).
    #[must_use]
    pub fn new(config: IrohNodeConfig) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(None)),
        }
    }

    /// Create with default configuration.
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(IrohNodeConfig::default())
    }

    /// Create and start a node with in-memory store.
    ///
    /// Convenience constructor that creates a default-configured node
    /// and starts it immediately. Most callers should use this.
    pub async fn memory() -> Result<Arc<Self>, IrohError> {
        let node = Arc::new(Self::with_defaults());
        node.start().await?;
        Ok(node)
    }

    /// Start the Iroh node.
    ///
    /// Initializes the endpoint, store, and protocol router.
    pub async fn start(&self) -> Result<(), IrohError> {
        let mut state = self.state.write().await;

        if state.is_some() {
            debug!("Iroh node already running");
            return Ok(());
        }

        info!("Starting Iroh node");

        // Create endpoint
        let endpoint = Endpoint::builder()
            .bind()
            .await
            .map_err(|e| IrohError::EndpointInit(e.to_string()))?;

        let endpoint_id = endpoint.id();
        info!(?endpoint_id, "Iroh endpoint bound");

        // Create in-memory store
        let store = MemStore::new();
        debug!("In-memory blob store initialized");

        // Create blob protocol handler
        let blobs = BlobsProtocol::new(&store, None);

        // Build router to accept blob protocol connections
        let router = Router::builder(endpoint.clone())
            .accept(iroh_blobs::ALPN, blobs)
            .spawn();

        info!("Iroh router started, accepting blob connections");

        *state = Some(RunningNode {
            endpoint,
            store,
            router,
        });
        drop(state);

        Ok(())
    }

    /// Stop the Iroh node gracefully.
    pub async fn stop(&self) -> Result<(), IrohError> {
        let running = {
            let mut state = self.state.write().await;
            state.take()
        };

        if let Some(running) = running {
            info!("Stopping Iroh node");

            running
                .router
                .shutdown()
                .await
                .map_err(|e| IrohError::Shutdown(e.to_string()))?;

            running.endpoint.close().await;
            info!("Iroh node stopped");
        } else {
            warn!("Iroh node was not running");
        }

        Ok(())
    }

    /// Check if the node is running.
    pub async fn is_running(&self) -> bool {
        self.state.read().await.is_some()
    }

    /// Get the node's endpoint (for creating connections).
    pub async fn endpoint(&self) -> Result<Endpoint, IrohError> {
        let state = self.state.read().await;
        state
            .as_ref()
            .map(|s| s.endpoint.clone())
            .ok_or(IrohError::NodeNotRunning)
    }

    /// Get the blob store (for adding/reading blobs).
    pub async fn store(&self) -> Result<MemStore, IrohError> {
        let state = self.state.read().await;
        state
            .as_ref()
            .map(|s| s.store.clone())
            .ok_or(IrohError::NodeNotRunning)
    }

    /// Get the endpoint ID (public key).
    pub async fn endpoint_id(&self) -> Result<EndpointId, IrohError> {
        let endpoint = self.endpoint().await?;
        Ok(endpoint.id())
    }
}

impl Drop for IrohNode {
    fn drop(&mut self) {
        // Note: Async cleanup should be done via stop() before dropping
        // This is just a safety warning
        if self.state.try_read().map(|s| s.is_some()).unwrap_or(false) {
            warn!("IrohNode dropped while still running. Call stop() for graceful shutdown.");
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults() {
        let config = IrohNodeConfig::default();
        assert!(config.relay_mode);

        let direct = IrohNodeConfig::direct_only();
        assert!(!direct.relay_mode);
    }

    #[tokio::test]
    async fn node_lifecycle() {
        let node = IrohNode::with_defaults();
        assert!(!node.is_running().await);

        // Start
        node.start().await.unwrap();
        assert!(node.is_running().await);

        // Can get endpoint and store
        let _endpoint = node.endpoint().await.unwrap();
        let _store = node.store().await.unwrap();
        let _endpoint_id = node.endpoint_id().await.unwrap();

        // Stop
        node.stop().await.unwrap();
        assert!(!node.is_running().await);

        // Operations fail when stopped
        assert!(node.endpoint().await.is_err());
        assert!(node.store().await.is_err());
    }

    #[tokio::test]
    async fn node_double_start() {
        let node = IrohNode::with_defaults();

        node.start().await.unwrap();
        // Second start should be idempotent
        node.start().await.unwrap();

        assert!(node.is_running().await);
        node.stop().await.unwrap();
    }

    #[tokio::test]
    async fn node_memory_constructor() {
        let node = IrohNode::memory().await.unwrap();
        assert!(node.is_running().await);

        let _endpoint = node.endpoint().await.unwrap();
        let _store = node.store().await.unwrap();

        node.stop().await.unwrap();
        assert!(!node.is_running().await);
    }

    #[tokio::test]
    async fn node_double_stop() {
        let node = IrohNode::with_defaults();

        node.start().await.unwrap();
        node.stop().await.unwrap();
        // Second stop should be safe
        node.stop().await.unwrap();

        assert!(!node.is_running().await);
    }
}
