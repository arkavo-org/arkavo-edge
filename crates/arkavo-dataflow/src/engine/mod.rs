pub mod executor;
pub mod metrics;
pub mod router;
#[cfg(feature = "sandbox")]
pub mod sandbox;

use crate::dsl::Blueprint;
use anyhow::Result;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum Message {
    Data {
        id: Uuid,
        payload: serde_json::Value,
        metadata: MessageMetadata,
    },
    Control(ControlMessage),
}

#[derive(Debug, Clone)]
pub struct MessageMetadata {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub source: String,
    pub trace_id: Uuid,
    pub headers: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub enum ControlMessage {
    Start,
    Stop,
    Flush,
    Metrics,
}

#[derive(Clone, Debug)]
pub struct Pipeline {
    id: Uuid,
    name: String,
    blueprint: Arc<Blueprint>,
    nodes: Arc<DashMap<String, executor::NodeExecutor>>,
    channels: Arc<DashMap<String, mpsc::Sender<Message>>>,
    status: Arc<RwLock<PipelineStatus>>,
    config: Arc<crate::DataflowConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PipelineStatus {
    Created,
    Running,
    Stopped,
    Error,
}

impl Pipeline {
    pub fn from_blueprint(
        blueprint: Blueprint,
        config: Arc<crate::DataflowConfig>,
    ) -> Result<Self> {
        let id = Uuid::new_v4();
        let name = blueprint.name.clone().unwrap_or_else(|| id.to_string());

        let pipeline = Self {
            id,
            name,
            blueprint: Arc::new(blueprint),
            nodes: Arc::new(DashMap::new()),
            channels: Arc::new(DashMap::new()),
            status: Arc::new(RwLock::new(PipelineStatus::Created)),
            config,
        };

        pipeline.initialize_nodes()?;
        pipeline.initialize_connections()?;

        Ok(pipeline)
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn name(&self) -> String {
        self.name.clone()
    }

    pub async fn start(&mut self) -> Result<()> {
        let mut status = self.status.write().await;
        if *status != PipelineStatus::Created && *status != PipelineStatus::Stopped {
            return Err(anyhow::anyhow!("Pipeline is not in a startable state"));
        }

        *status = PipelineStatus::Running;
        drop(status);

        // Start all node executors
        for entry in self.nodes.iter() {
            entry.value().start().await?;
        }

        // Send start control message to all nodes
        self.broadcast_control(ControlMessage::Start).await?;

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<()> {
        let mut status = self.status.write().await;
        if *status != PipelineStatus::Running {
            return Err(anyhow::anyhow!("Pipeline is not running"));
        }

        *status = PipelineStatus::Stopped;
        drop(status);

        // Send stop control message to all nodes
        self.broadcast_control(ControlMessage::Stop).await?;

        // Stop all node executors
        for entry in self.nodes.iter() {
            entry.value().stop().await?;
        }

        Ok(())
    }

    pub async fn status(&self) -> PipelineStatus {
        *self.status.read().await
    }

    pub async fn send_to_node(&self, node_id: &str, message: Message) -> Result<()> {
        if let Some(sender) = self.channels.get(node_id) {
            sender
                .send(message)
                .await
                .map_err(|_| anyhow::anyhow!("Failed to send message to node {node_id}"))?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Node {node_id} not found"))
        }
    }

    fn initialize_nodes(&self) -> Result<()> {
        for node in &self.blueprint.nodes {
            let (tx, rx) = mpsc::channel(self.config.max_queue_size);

            let executor = executor::NodeExecutor::new(node.clone(), rx, self.config.clone())?;

            self.nodes.insert(node.id.clone(), executor);
            self.channels.insert(node.id.clone(), tx);
        }

        Ok(())
    }

    fn initialize_connections(&self) -> Result<()> {
        // Set up routing based on links
        for link in &self.blueprint.links {
            if let Some(mut from_executor) = self.nodes.get_mut(&link.from) {
                if let Some(to_sender) = self.channels.get(&link.to) {
                    from_executor.add_output(link.clone(), to_sender.clone());
                }
            }
        }

        Ok(())
    }

    async fn broadcast_control(&self, message: ControlMessage) -> Result<()> {
        let control_msg = Message::Control(message);

        for entry in self.channels.iter() {
            let _ = entry.value().send(control_msg.clone()).await;
        }

        Ok(())
    }

    pub async fn get_metrics(&self) -> metrics::PipelineMetrics {
        let mut node_metrics = std::collections::HashMap::new();

        for entry in self.nodes.iter() {
            let metrics = entry.value().get_metrics().await;
            node_metrics.insert(entry.key().clone(), metrics);
        }

        metrics::PipelineMetrics {
            pipeline_id: self.id,
            pipeline_name: self.name.clone(),
            status: *self.status.read().await,
            node_metrics,
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn to_blueprint(&self) -> Blueprint {
        (*self.blueprint).clone()
    }
}

impl Message {
    pub fn new_data(payload: serde_json::Value, source: impl Into<String>) -> Self {
        Self::Data {
            id: Uuid::new_v4(),
            payload,
            metadata: MessageMetadata {
                timestamp: chrono::Utc::now(),
                source: source.into(),
                trace_id: Uuid::new_v4(),
                headers: std::collections::HashMap::new(),
            },
        }
    }

    pub fn with_trace_id(mut self, trace_id: Uuid) -> Self {
        if let Message::Data {
            ref mut metadata, ..
        } = self
        {
            metadata.trace_id = trace_id;
        }
        self
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        if let Message::Data {
            ref mut metadata, ..
        } = self
        {
            metadata.headers.insert(key.into(), value.into());
        }
        self
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;
    use crate::dsl::{Blueprint, Link, Node};

    #[tokio::test]
    async fn test_pipeline_lifecycle() {
        let mut blueprint = Blueprint::new("test");
        blueprint.add_node(Node::source("input"));
        blueprint.add_node(Node::sink("output"));
        blueprint.add_link(Link::new("input", "output"));

        let config = Arc::new(crate::DataflowConfig::default());
        let mut pipeline = Pipeline::from_blueprint(blueprint, config).unwrap();

        assert_eq!(pipeline.status().await, PipelineStatus::Created);

        pipeline.start().await.unwrap();
        assert_eq!(pipeline.status().await, PipelineStatus::Running);

        pipeline.stop().await.unwrap();
        assert_eq!(pipeline.status().await, PipelineStatus::Stopped);
    }
}
