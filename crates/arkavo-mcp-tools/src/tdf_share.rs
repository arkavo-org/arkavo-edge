//! Composite TDF P2P share/receive MCP tools.
//!
//! Orchestrates encrypt + stage + ticket exchange for interagent
//! secure data sharing over Iroh P2P transport.

use crate::server::Tool;
use arkavo_mcp::ToolSchema;
use arkavo_tdf::{OpenTdfService, PolicyBuilder, TdfEncryptor, TdfManifest};
use arkavo_tdf_iroh::{IrohNode, IrohTransport, RETENTION};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::iroh_data::{Fetched, fetch, reference_schema};

/// A TDF manifest travels as its JSON serialization.
const TDF_MANIFEST: &str = "application/json";

/// MCP tool for securely sharing TDF-encrypted data with another agent via Iroh P2P.
///
/// Orchestrates: read file -> encrypt with TDF -> stage to Iroh -> return a
/// data-plane reference + metadata. The caller sends the reference to the
/// target agent, for `tdf_receive` or as a `tdf.share` offer.
pub struct TdfShareTool {
    schema: ToolSchema,
    iroh_node: Option<Arc<IrohNode>>,
}

impl TdfShareTool {
    pub fn new(iroh_node: Option<Arc<IrohNode>>) -> Self {
        Self {
            schema: ToolSchema {
                name: "tdf_share".to_string(),
                aliases: Some(vec!["tdf_send".to_string()]),
                description: "Encrypt a file with TDF and stage it on the Iroh P2P network \
                    for an hour. Returns a reference to the encrypted data (name, ticket, content \
                    hash, size, media type) and its policy, for a target agent to fetch with \
                    tdf_receive or to offer via tdf.share RPC."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "input_path": {
                            "type": "string",
                            "description": "Path to the file to encrypt and share"
                        },
                        "namespace": {
                            "type": "string",
                            "description": "Attribute namespace FQN",
                            "default": "https://arkavo.net/attr/sensitivity"
                        },
                        "values": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Attribute values for access control",
                            "default": ["internal"]
                        },
                        "kas_url": {
                            "type": "string",
                            "description": "KAS URL for key wrapping",
                            "default": "https://100.arkavo.net"
                        }
                    },
                    "required": ["input_path"]
                }),
            },
            iroh_node,
        }
    }
}

#[async_trait]
impl Tool for TdfShareTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let input_path = args["input_path"]
            .as_str()
            .ok_or_else(|| crate::ToolError::InvalidParams("Missing input_path".to_string()))?;

        // Reject path traversal attempts
        let path = std::path::Path::new(input_path);
        if path
            .components()
            .any(|c| c == std::path::Component::ParentDir)
        {
            return Err(crate::ToolError::InvalidParams(
                "Path traversal (.. components) not allowed".to_string(),
            ));
        }

        let namespace = args["namespace"]
            .as_str()
            .unwrap_or("https://arkavo.net/attr/sensitivity");

        let values: Vec<&str> = args["values"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_else(|| vec!["internal"]);

        let kas_url = args["kas_url"].as_str().unwrap_or("https://100.arkavo.net");

        // Read input file
        let plaintext = tokio::fs::read(input_path)
            .await
            .map_err(crate::ToolError::Io)?;

        // Encrypt with TDF
        let service = OpenTdfService::with_kas_url(kas_url);
        let policy = PolicyBuilder::new()
            .attribute(namespace, &values)
            .build()
            .map_err(|e| crate::ToolError::Execution(format!("Policy error: {e}")))?;

        let manifest = service
            .encrypt(&plaintext, &policy)
            .await
            .map_err(|e| crate::ToolError::Execution(format!("Encryption failed: {e}")))?;

        // Serialize manifest to JSON for transport
        let manifest_json = serde_json::to_vec(&manifest)?;

        // Stage to Iroh
        let node = match &self.iroh_node {
            Some(n) => n.clone(),
            None => IrohNode::memory().await.map_err(|e| {
                crate::ToolError::Execution(format!("Failed to create Iroh node: {e}"))
            })?,
        };
        let transport = IrohTransport::new(node);

        let name = format!(
            "{}.tdf.json",
            path.file_name()
                .map_or_else(|| "data".into(), |n| n.to_string_lossy())
        );
        let reference = transport
            .stage_ref(&name, TDF_MANIFEST, &manifest_json, RETENTION)
            .await
            .map_err(|e| crate::ToolError::Execution(format!("Failed to stage: {e}")))?;

        Ok(json!({
            "success": true,
            "input_path": input_path,
            "plaintext_size": plaintext.len(),
            "reference": reference,
            "kas_url": kas_url,
            "policy_attributes": [namespace],
            "algorithm": manifest.encryption_information.method.algorithm
        }))
    }
}

/// MCP tool for receiving and decrypting TDF-encrypted data from Iroh P2P.
///
/// Orchestrates: fetch by reference from Iroh -> parse manifest -> write
/// encrypted manifest to disk.
/// Full decryption requires KAS rewrap (handled separately).
pub struct TdfReceiveTool {
    schema: ToolSchema,
    iroh_node: Option<Arc<IrohNode>>,
}

impl TdfReceiveTool {
    pub fn new(iroh_node: Option<Arc<IrohNode>>) -> Self {
        Self {
            schema: ToolSchema {
                name: "tdf_receive".to_string(),
                aliases: Some(vec!["tdf_accept".to_string()]),
                description: "Fetch TDF-encrypted data from the Iroh P2P network by the \
                    reference tdf_share returned. Retrieves and saves the TDF manifest for later \
                    decryption via KAS."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "reference": reference_schema(
                            "The reference tdf_share returned, as it was given"
                        ),
                        "ticket": {
                            "type": "string",
                            "description": "A bare Iroh blob ticket, as a tdf.share offer lists it"
                        },
                        "output_path": {
                            "type": "string",
                            "description": "Path to write the TDF manifest file"
                        }
                    },
                    "required": ["output_path"]
                }),
            },
            iroh_node,
        }
    }
}

#[async_trait]
impl Tool for TdfReceiveTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let output_path = args["output_path"]
            .as_str()
            .ok_or_else(|| crate::ToolError::InvalidParams("Missing output_path".to_string()))?;

        // Reject path traversal attempts
        let path = std::path::Path::new(output_path);
        if path
            .components()
            .any(|c| c == std::path::Component::ParentDir)
        {
            return Err(crate::ToolError::InvalidParams(
                "Path traversal (.. components) not allowed".to_string(),
            ));
        }

        // Fetch from Iroh
        let node = match &self.iroh_node {
            Some(n) => n.clone(),
            None => IrohNode::memory().await.map_err(|e| {
                crate::ToolError::Execution(format!("Failed to create Iroh node: {e}"))
            })?,
        };
        let transport = IrohTransport::new(node);
        let Fetched {
            data, content_hash, ..
        } = fetch(&transport, &args).await?;

        // Parse as TDF manifest to validate
        let manifest: TdfManifest = serde_json::from_slice(&data)
            .map_err(|e| crate::ToolError::Execution(format!("Invalid TDF manifest: {e}")))?;

        // Write manifest to output
        tokio::fs::write(output_path, &data)
            .await
            .map_err(crate::ToolError::Io)?;

        let key_access_count = manifest.encryption_information.key_access.len();
        let kas_url = manifest
            .encryption_information
            .key_access
            .first()
            .map(|ka| ka.url.as_str())
            .unwrap_or("unknown");

        Ok(json!({
            "success": true,
            "output_path": output_path,
            "size_bytes": data.len(),
            "version": manifest.version,
            "algorithm": manifest.encryption_information.method.algorithm,
            "key_access_count": key_access_count,
            "kas_url": kas_url,
            "content_hash": content_hash
        }))
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[tokio::test]
    async fn test_share_tool_schema() {
        let tool = TdfShareTool::new(None);
        let schema = tool.schema();

        assert_eq!(schema.name, "tdf_share");
        assert!(schema.description.contains("Encrypt"));
        assert!(schema.description.contains("Iroh"));
    }

    #[tokio::test]
    async fn test_receive_tool_schema() {
        let tool = TdfReceiveTool::new(None);
        let schema = tool.schema();

        assert_eq!(schema.name, "tdf_receive");
        assert!(schema.description.contains("Fetch"));
    }

    #[tokio::test]
    async fn test_share_missing_input() {
        let tool = TdfShareTool::new(None);
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_receive_missing_ticket() {
        let tool = TdfReceiveTool::new(None);
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_share_rejects_path_traversal() {
        let tool = TdfShareTool::new(None);
        let result = tool
            .execute(json!({"input_path": "../../../etc/passwd"}))
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Path traversal"), "Got: {err}");
    }

    #[tokio::test]
    async fn test_receive_rejects_path_traversal() {
        let tool = TdfReceiveTool::new(None);
        let result = tool
            .execute(json!({"ticket": "blobaaaa1234", "output_path": "../../tmp/evil.json"}))
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Path traversal"), "Got: {err}");
    }

    /// MCP-019: the encrypted manifest is staged as a reference named after
    /// the file, and the reference fetches it back for tdf_receive.
    #[spec("MCP-019")]
    #[tokio::test]
    async fn test_share_and_receive_roundtrip() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let input_path = temp_dir.path().join("secret.txt");
        let output_path = temp_dir.path().join("received.tdf.json");

        tokio::fs::write(&input_path, b"Top secret agent data")
            .await
            .unwrap();

        // Share: encrypt + stage
        let node = IrohNode::memory().await.unwrap();
        let share_tool = TdfShareTool::new(Some(node.clone()));

        let share_result = share_tool
            .execute(json!({
                "input_path": input_path.to_str().unwrap()
            }))
            .await
            .unwrap();

        assert!(share_result["success"].as_bool().unwrap());
        let reference = &share_result["reference"];
        assert_eq!(reference["name"], "secret.txt.tdf.json");
        assert_eq!(reference["media_type"], TDF_MANIFEST);
        assert!(!reference["ticket"].as_str().unwrap().is_empty());
        assert!(share_result.get("ticket").is_none());

        // Receive: fetch + save manifest
        let receive_tool = TdfReceiveTool::new(Some(node.clone()));

        let receive_result = receive_tool
            .execute(json!({
                "reference": reference,
                "output_path": output_path.to_str().unwrap()
            }))
            .await
            .unwrap();

        assert!(receive_result["success"].as_bool().unwrap());
        assert_eq!(receive_result["version"], "3.0.0");
        assert_eq!(receive_result["algorithm"], "AES-256-GCM");
        assert_eq!(receive_result["content_hash"], reference["content_hash"]);
        assert_eq!(receive_result["size_bytes"], reference["size"]);

        // Verify file was written
        let saved = tokio::fs::read_to_string(&output_path).await.unwrap();
        let manifest: TdfManifest = serde_json::from_str(&saved).unwrap();
        assert_eq!(manifest.version, "3.0.0");

        node.stop().await.unwrap();
    }

    /// MCP-019: a reference that does not match the staged blob is refused
    /// and nothing is written; a bare ticket from a tdf.share offer still
    /// fetches.
    #[spec("MCP-019")]
    #[tokio::test]
    async fn test_receive_checks_the_reference() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let input_path = temp_dir.path().join("secret.txt");
        tokio::fs::write(&input_path, b"Top secret agent data")
            .await
            .unwrap();
        let node = IrohNode::memory().await.unwrap();
        let share_result = TdfShareTool::new(Some(node.clone()))
            .execute(json!({"input_path": input_path.to_str().unwrap()}))
            .await
            .unwrap();
        let receive_tool = TdfReceiveTool::new(Some(node.clone()));

        let mut understated = share_result["reference"].clone();
        understated["size"] = json!(16);
        let refused = temp_dir.path().join("refused.tdf.json");
        let err = receive_tool
            .execute(json!({
                "reference": understated,
                "output_path": refused.to_str().unwrap()
            }))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("bytes held, 16 referenced"), "{err}");
        assert!(!refused.exists());

        let offered = temp_dir.path().join("offered.tdf.json");
        let receive_result = receive_tool
            .execute(json!({
                "ticket": share_result["reference"]["ticket"],
                "output_path": offered.to_str().unwrap()
            }))
            .await
            .unwrap();
        assert_eq!(
            receive_result["content_hash"],
            share_result["reference"]["content_hash"]
        );
        assert!(offered.exists());

        node.stop().await.unwrap();
    }
}
