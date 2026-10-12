//! In-memory Iroh P2P data tools for inter-agent data sharing.
//!
//! Unlike the file-based `TdfStageTool`/`TdfFetchTool`, these tools operate on
//! string data directly — no file I/O. They use the agent's shared `IrohNode`
//! so staged blobs are fetchable by other agents in the mesh.

use crate::server::Tool;
use arkavo_mcp::ToolSchema;
use arkavo_tdf_iroh::{DataRef, FetchPolicy, IrohNode, IrohTransport, RETENTION};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

/// Media type of data staged from a JSON string argument.
const TEXT: &str = "text/plain; charset=utf-8";
/// Media type of data staged from a JSON object or array argument.
const JSON: &str = "application/json";

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
/// and returns a data-plane reference any other agent can fetch it by.
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
                description: "Stage data on the Iroh P2P network for sharing with other agents \
                    for an hour. Returns a reference (name, ticket, content hash, size, media \
                    type); pass the whole reference to the recipient, who fetches it with \
                    iroh_fetch."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "data": {
                            "description": "The data to stage (string or JSON object)"
                        },
                        "name": {
                            "type": "string",
                            "description": "What the data is to the recipient",
                            "default": "data"
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
        let data_value = args
            .get("data")
            .ok_or_else(|| crate::ToolError::InvalidParams("Missing data parameter".to_string()))?;
        let name = args["name"].as_str().unwrap_or("data");

        // Accept string directly or serialize objects/arrays to JSON
        let (data_string, media_type) = match data_value.as_str() {
            Some(s) => (s.to_string(), TEXT),
            None => (
                serde_json::to_string(data_value).map_err(|e| {
                    crate::ToolError::InvalidParams(format!("Cannot serialize data: {e}"))
                })?,
                JSON,
            ),
        };

        let transport = IrohTransport::new(self.iroh_node.clone());
        let reference = transport
            .stage_ref(name, media_type, data_string.as_bytes(), RETENTION)
            .await
            .map_err(|e| crate::ToolError::Execution(format!("Iroh stage failed: {e}")))?;

        tracing::info!(
            size_bytes = reference.size,
            ticket_len = reference.ticket.len(),
            "Staged data on Iroh P2P"
        );

        Ok(json!({
            "success": true,
            "reference": reference
        }))
    }
}

/// MCP tool for fetching data from the Iroh P2P network by reference.
///
/// Takes the reference another agent staged, fetches exactly the blob it
/// names, and returns the data as a string in the response.
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
                    "Fetch data from the Iroh P2P network using the reference another agent \
                    staged it under. Returns the data as a string."
                        .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "reference": reference_schema(
                            "The reference from the staging agent, as it was given"
                        ),
                        "ticket": {
                            "type": "string",
                            "description": "A bare Iroh blob ticket, only from an agent that sends no reference"
                        }
                    }
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
        let transport = IrohTransport::new(self.iroh_node.clone());
        let Fetched {
            data, reference, ..
        } = fetch(&transport, &args).await?;

        let text = String::from_utf8(data.clone()).unwrap_or_else(|_| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(&data)
        });

        tracing::info!(size_bytes = data.len(), "Fetched data from Iroh P2P");

        let mut result = json!({
            "success": true,
            "data": text,
            "size_bytes": data.len()
        });
        if let Some(reference) = reference {
            result["name"] = json!(reference.name);
            result["media_type"] = json!(reference.media_type);
        }
        Ok(result)
    }
}

/// JSON schema of a [`DataRef`] argument.
pub(crate) fn reference_schema(description: &str) -> Value {
    json!({
        "type": "object",
        "description": description,
        "properties": {
            "name": { "type": "string" },
            "ticket": { "type": "string" },
            "content_hash": { "type": "string" },
            "size": { "type": "integer" },
            "media_type": { "type": "string" }
        },
        "required": ["name", "ticket", "content_hash", "size", "media_type"]
    })
}

/// What a fetch by [`fetch`] produced.
pub(crate) struct Fetched {
    pub data: Vec<u8>,
    pub content_hash: String,
    /// The reference fetched by; `None` for a bare ticket.
    pub reference: Option<DataRef>,
}

/// The bytes `args` point at, by `reference`, or by a bare `ticket` from an
/// agent that predates references, which has no size to check before the
/// download.
pub(crate) async fn fetch(transport: &IrohTransport, args: &Value) -> crate::Result<Fetched> {
    let failed = |e: arkavo_tdf_iroh::IrohError| {
        crate::ToolError::Execution(format!("Iroh fetch failed: {e}"))
    };
    if let Some(reference) = args.get("reference").filter(|r| !r.is_null()) {
        let reference: DataRef = serde_json::from_value(reference.clone())
            .map_err(|e| crate::ToolError::InvalidParams(format!("Invalid reference: {e}")))?;
        let data = transport
            .fetch_ref(&reference, FetchPolicy::default())
            .await
            .map_err(failed)?;
        return Ok(Fetched {
            data,
            content_hash: reference.content_hash.clone(),
            reference: Some(reference),
        });
    }
    let ticket_str = args["ticket"].as_str().ok_or_else(|| {
        crate::ToolError::InvalidParams("Missing reference parameter".to_string())
    })?;
    let ticket: arkavo_tdf_iroh::IrohTicket = ticket_str
        .parse()
        .map_err(|e| crate::ToolError::InvalidParams(format!("Invalid Iroh ticket: {e}")))?;
    let data = transport.fetch_bytes(&ticket).await.map_err(failed)?;
    Ok(Fetched {
        data,
        content_hash: ticket.hash().to_string(),
        reference: None,
    })
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

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

    /// MCP-018: staged data comes back as a reference naming its size,
    /// hash and media type, and the reference fetches the same data.
    #[spec("MCP-018")]
    #[tokio::test]
    async fn test_stage_and_fetch_roundtrip() {
        let node = IrohNode::memory().await.unwrap();
        let stage_tool = IrohStageTool::new(node.clone());
        let fetch_tool = IrohFetchTool::new(node.clone());

        let test_data = r#"{"episode": "summary", "reward": -12.5}"#;

        let stage_result = stage_tool
            .execute(json!({"data": test_data, "name": "episode.json"}))
            .await
            .unwrap();

        assert!(stage_result["success"].as_bool().unwrap());
        let reference: DataRef = serde_json::from_value(stage_result["reference"].clone()).unwrap();
        assert_eq!(reference.name, "episode.json");
        assert_eq!(reference.size, test_data.len() as u64);
        assert_eq!(reference.media_type, TEXT);
        assert_eq!(reference.content_hash.len(), 64);
        assert!(stage_result.get("ticket").is_none());

        let fetch_result = fetch_tool
            .execute(json!({"reference": stage_result["reference"]}))
            .await
            .unwrap();

        assert!(fetch_result["success"].as_bool().unwrap());
        assert_eq!(fetch_result["data"].as_str().unwrap(), test_data);
        assert_eq!(
            fetch_result["size_bytes"].as_u64().unwrap(),
            test_data.len() as u64
        );
        assert_eq!(fetch_result["name"], "episode.json");
        assert_eq!(fetch_result["media_type"], TEXT);

        node.stop().await.unwrap();
    }

    /// MCP-018: an object is staged as JSON under the default name.
    #[spec("MCP-018")]
    #[tokio::test]
    async fn test_stage_object_is_json() {
        let node = IrohNode::memory().await.unwrap();
        let stage_result = IrohStageTool::new(node.clone())
            .execute(json!({"data": {"turn": 12}}))
            .await
            .unwrap();
        assert_eq!(stage_result["reference"]["media_type"], JSON);
        assert_eq!(stage_result["reference"]["name"], "data");
        assert_eq!(stage_result["reference"]["size"], 11);
        node.stop().await.unwrap();
    }

    /// MCP-018: a reference whose size or hash does not match the blob is
    /// refused; a bare ticket from an agent without references still
    /// fetches.
    #[spec("MCP-018")]
    #[tokio::test]
    async fn test_fetch_checks_the_reference() {
        let node = IrohNode::memory().await.unwrap();
        let stage_result = IrohStageTool::new(node.clone())
            .execute(json!({"data": "full state"}))
            .await
            .unwrap();
        let fetch_tool = IrohFetchTool::new(node.clone());

        let mut understated = stage_result["reference"].clone();
        understated["size"] = json!(4);
        let err = fetch_tool
            .execute(json!({"reference": understated}))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("10 bytes held"), "{err}");

        let mut other = stage_result["reference"].clone();
        other["content_hash"] = json!("0".repeat(64));
        let err = fetch_tool
            .execute(json!({"reference": other}))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("different blob"), "{err}");

        let err = fetch_tool
            .execute(json!({"reference": {"ticket": "x"}}))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("Invalid reference"), "{err}");

        let ticket = stage_result["reference"]["ticket"].clone();
        let fetch_result = fetch_tool.execute(json!({"ticket": ticket})).await.unwrap();
        assert_eq!(fetch_result["data"], "full state");
        assert!(fetch_result.get("media_type").is_none());

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
