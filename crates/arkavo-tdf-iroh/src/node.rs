//! Iroh node lifecycle management.
//!
//! Manages an embedded Iroh endpoint with blob protocol support.

use iroh::Endpoint;
use iroh::endpoint::presets::N0;
use iroh::protocol::Router;
use iroh_base::EndpointId;
use iroh_blobs::BlobsProtocol;
use iroh_blobs::store::GcConfig;
use iroh_blobs::store::mem::{MemStore, Options};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::error::IrohError;
use crate::retention::{Retention, sweep};

/// Configuration for the Iroh node.
#[derive(Debug, Clone)]
pub struct IrohNodeConfig {
    /// Enable relay mode for NAT traversal (default: true)
    pub relay_mode: bool,
    /// How often expired retained blobs are untagged and unreferenced blobs
    /// are collected.
    pub gc_interval: Duration,
    /// Bytes that may be staged for other parties at once (IROH-010).
    pub staged_limit: u64,
}

impl Default for IrohNodeConfig {
    fn default() -> Self {
        Self {
            relay_mode: true,
            gc_interval: Duration::from_mins(1),
            staged_limit: 1 << 30,
        }
    }
}

impl IrohNodeConfig {
    /// Create a new config with relay mode disabled (direct connections only).
    #[must_use]
    pub fn direct_only() -> Self {
        Self {
            relay_mode: false,
            ..Self::default()
        }
    }
}

/// State of a running Iroh node.
struct RunningNode {
    endpoint: Endpoint,
    store: MemStore,
    router: Router,
    sweeper: tokio::task::JoinHandle<()>,
}

impl Drop for RunningNode {
    fn drop(&mut self) {
        self.sweeper.abort();
    }
}

/// Embedded Iroh node for blob storage and transfer.
///
/// Manages the lifecycle of:
/// - Iroh endpoint (P2P connections)
/// - Blob store (in-memory)
/// - Protocol router (handles blob requests)
pub struct IrohNode {
    config: IrohNodeConfig,
    state: Arc<RwLock<Option<RunningNode>>>,
    retention: Arc<Retention>,
}

impl IrohNode {
    /// Create a new Iroh node (not yet started).
    #[must_use]
    pub fn new(config: IrohNodeConfig) -> Self {
        Self {
            retention: Arc::new(Retention::new(config.staged_limit)),
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

        // Create endpoint with iroh's default n0.computer relay/DNS preset.
        let endpoint = Endpoint::builder(N0)
            .bind()
            .await
            .map_err(|e| IrohError::EndpointInit(e.to_string()))?;

        let endpoint_id = endpoint.id();
        info!(?endpoint_id, "Iroh endpoint bound");

        // Collection frees blobs whose retention ended; blobs staged with a
        // persistent tag, or protected by a temp tag, are kept.
        let store = MemStore::new_with_opts(Options {
            gc_config: Some(GcConfig {
                interval: self.config.gc_interval,
                add_protected: None,
            }),
        });
        debug!("In-memory blob store initialized");
        let sweeper = {
            let (store, retention) = (store.clone(), self.retention.clone());
            let mut every = tokio::time::interval(self.config.gc_interval);
            tokio::spawn(async move {
                loop {
                    every.tick().await;
                    sweep(&store, &retention).await;
                }
            })
        };

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
            sweeper,
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

    /// The blobs this node keeps for other parties.
    pub(crate) fn retention(&self) -> &Retention {
        &self.retention
    }

    /// Bytes currently staged for other parties.
    pub fn staged_bytes(&self) -> u64 {
        self.retention.staged_bytes()
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
    use arkavo_test_macros::spec;

    #[spec("IROH-006")]
    #[test]
    fn config_defaults() {
        let config = IrohNodeConfig::default();
        assert!(config.relay_mode);

        let direct = IrohNodeConfig::direct_only();
        assert!(!direct.relay_mode);
    }

    #[spec("IROH-005")]
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

    #[spec("IROH-005")]
    #[tokio::test]
    async fn node_double_stop() {
        let node = IrohNode::with_defaults();

        node.start().await.unwrap();
        node.stop().await.unwrap();
        // Second stop should be safe
        node.stop().await.unwrap();

        assert!(!node.is_running().await);
    }

    #[spec("IROH-006")]
    #[tokio::test]
    async fn node_with_direct_only_config_starts() {
        let node = IrohNode::new(IrohNodeConfig::direct_only());
        assert!(!node.is_running().await);

        node.start().await.unwrap();
        assert!(node.is_running().await);

        // Configured node binds and exposes endpoint and store
        let _endpoint = node.endpoint().await.unwrap();
        let _store = node.store().await.unwrap();

        node.stop().await.unwrap();
        assert!(!node.is_running().await);
    }
}
