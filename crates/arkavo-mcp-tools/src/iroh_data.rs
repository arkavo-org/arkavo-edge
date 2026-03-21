//! In-memory Iroh P2P data tools for inter-agent data sharing.
//!
//! Unlike the file-based `TdfStageTool`/`TdfFetchTool`, these tools operate on
//! string data directly — no file I/O. They use the agent's shared `IrohNode`
//! so staged blobs are fetchable by other agents in the mesh.

use crate::server::Tool;
use arkavo_mcp::ToolSchema;
use arkavo_tdf_iroh::{IrohNode, IrohTransport};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

/// Register Iroh P2P data tools into a tool registry.
pub fn register_iroh_tools(registry: &mut crate::ToolRegistry, iroh_node: Arc<IrohNode>) {
    registry.register(
        "iroh_stage",
        Box::new(IrohStageTool::new(iroh_node.clone())),
    );
    registry.register("iroh_fetch", Box::new(IrohFetchTool::new(iroh_node)));
}

/// MCP tool for staging string data to the Iroh P2P network.
///
/// Takes data as a string parameter, stages it on the shared Iroh node,
/// and returns a ticket that any other agent can use to fetch the data.
pub struct IrohStageTool {
    schema: ToolSchema,
    iroh_node: Arc<IrohNode>,
}

impl IrohStageTool {
    pub fn new(iroh_node: Arc<IrohNode>) -> Self {
        Self {
            schema: ToolSchema {
                name: "iroh_stage".to_string(),
                aliases: None,
                description: "Stage data on the Iroh P2P network for sharing with other agents. \
                    Returns a ticket string that the recipient can use to fetch the data."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "data": {
                            "type": "string",
                            "description": "The data to stage (text or JSON)"
                        }
                    },
                    "required": ["data"]
                }),
            },
            iroh_node,
        }
    }
}

#[async_trait]
impl Tool for IrohStageTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let data = args["data"]
            .as_str()
            .ok_or_else(|| crate::ToolError::InvalidParams("Missing data parameter".to_string()))?;

        let bytes = data.as_bytes();
        let transport = IrohTransport::new(self.iroh_node.clone());

        let ticket = transport
            .stage_bytes(bytes)
            .await
            .map_err(|e| crate::ToolError::Execution(format!("Iroh stage failed: {e}")))?;

        let hash = ticket.hash().to_string();
        let ticket_str = ticket.to_string();

        tracing::info!(
            size_bytes = bytes.len(),
            ticket_len = ticket_str.len(),
            "Staged data on Iroh P2P"
        );

        Ok(json!({
            "success": true,
            "ticket": ticket_str,
            "size_bytes": bytes.len(),
            "hash": hash
        }))
    }
}

/// MCP tool for fetching data from the Iroh P2P network using a ticket.
///
/// Takes a ticket string, fetches the blob from the Iroh network,
/// and returns the data as a string in the response.
pub struct IrohFetchTool {
    schema: ToolSchema,
    iroh_node: Arc<IrohNode>,
}

impl IrohFetchTool {
    pub fn new(iroh_node: Arc<IrohNode>) -> Self {
        Self {
            schema: ToolSchema {
                name: "iroh_fetch".to_string(),
                aliases: None,
                description:
                    "Fetch data from the Iroh P2P network using a ticket from another agent. \
                    Returns the data as a string."
                        .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "ticket": {
                            "type": "string",
                            "description": "Iroh blob ticket from the staging agent"
                        }
                    },
                    "required": ["ticket"]
                }),
            },
            iroh_node,
        }
    }
}

#[async_trait]
impl Tool for IrohFetchTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let ticket_str = args["ticket"].as_str().ok_or_else(|| {
            crate::ToolError::InvalidParams("Missing ticket parameter".to_string())
        })?;

        let ticket: arkavo_tdf_iroh::IrohTicket = ticket_str
            .parse()
            .map_err(|e| crate::ToolError::InvalidParams(format!("Invalid Iroh ticket: {e}")))?;

        let transport = IrohTransport::new(self.iroh_node.clone());

        let data = transport
            .fetch_bytes(&ticket)
            .await
            .map_err(|e| crate::ToolError::Execution(format!("Iroh fetch failed: {e}")))?;

        let text = String::from_utf8(data.clone()).unwrap_or_else(|_| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(&data)
        });

        tracing::info!(size_bytes = data.len(), "Fetched data from Iroh P2P");

        Ok(json!({
            "success": true,
            "data": text,
            "size_bytes": data.len()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stage_tool_schema() {
        let node = IrohNode::memory().await.unwrap();
        let tool = IrohStageTool::new(node);
        let schema = tool.schema();

        assert_eq!(schema.name, "iroh_stage");
        assert!(schema.description.contains("Stage"));
        assert!(schema.aliases.is_none());
    }

    #[tokio::test]
    async fn test_fetch_tool_schema() {
        let node = IrohNode::memory().await.unwrap();
        let tool = IrohFetchTool::new(node);
        let schema = tool.schema();

        assert_eq!(schema.name, "iroh_fetch");
        assert!(schema.description.contains("Fetch"));
    }

    #[tokio::test]
    async fn test_stage_missing_data() {
        let node = IrohNode::memory().await.unwrap();
        let tool = IrohStageTool::new(node);
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_missing_ticket() {
        let node = IrohNode::memory().await.unwrap();
        let tool = IrohFetchTool::new(node);
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_stage_and_fetch_roundtrip() {
        let node = IrohNode::memory().await.unwrap();
        let stage_tool = IrohStageTool::new(node.clone());
        let fetch_tool = IrohFetchTool::new(node.clone());

        let test_data = r#"{"episode": "summary", "reward": -12.5}"#;

        let stage_result = stage_tool
            .execute(json!({"data": test_data}))
            .await
            .unwrap();

        assert!(stage_result["success"].as_bool().unwrap());
        let ticket = stage_result["ticket"].as_str().unwrap();
        assert!(!ticket.is_empty());
        assert_eq!(
            stage_result["size_bytes"].as_u64().unwrap(),
            test_data.len() as u64
        );

        let fetch_result = fetch_tool.execute(json!({"ticket": ticket})).await.unwrap();

        assert!(fetch_result["success"].as_bool().unwrap());
        assert_eq!(fetch_result["data"].as_str().unwrap(), test_data);
        assert_eq!(
            fetch_result["size_bytes"].as_u64().unwrap(),
            test_data.len() as u64
        );

        node.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_fetch_invalid_ticket() {
        let node = IrohNode::memory().await.unwrap();
        let tool = IrohFetchTool::new(node);
        let result = tool.execute(json!({"ticket": "not-a-valid-ticket"})).await;
        assert!(result.is_err());
    }
}
