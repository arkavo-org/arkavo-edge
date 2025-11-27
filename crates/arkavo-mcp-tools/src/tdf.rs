//! TDF (Trusted Data Format) MCP tools for encryption and info operations.

use crate::server::Tool;
use arkavo_mcp::ToolSchema;
use arkavo_tdf::{OpenTdfService, PolicyBuilder, TdfEncryptor, TdfManifest};
use async_trait::async_trait;
use serde_json::{Value, json};

/// MCP tool for encrypting data using TDF format.
pub struct TdfEncryptTool {
    schema: ToolSchema,
}

impl TdfEncryptTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "tdf_encrypt".to_string(),
                aliases: Some(vec!["encrypt_tdf".to_string()]),
                description: "Encrypt data using OpenTDF format with AES-256-GCM. \
                    Creates a TDF manifest with encrypted payload and policy-based access control."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "input_path": {
                            "type": "string",
                            "description": "Path to the file to encrypt"
                        },
                        "output_path": {
                            "type": "string",
                            "description": "Optional output path. Defaults to <input>.tdf.json"
                        },
                        "namespace": {
                            "type": "string",
                            "description": "Attribute namespace FQN (e.g., https://arkavo.net/attr/sensitivity)",
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
        }
    }
}

impl Default for TdfEncryptTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TdfEncryptTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let input_path = args["input_path"]
            .as_str()
            .ok_or_else(|| crate::ToolError::InvalidParams("Missing input_path".to_string()))?;

        let namespace = args["namespace"]
            .as_str()
            .unwrap_or("https://arkavo.net/attr/sensitivity");

        let values: Vec<&str> = args["values"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_else(|| vec!["internal"]);

        let kas_url = args["kas_url"].as_str().unwrap_or("https://100.arkavo.net");

        let output_path = args["output_path"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| format!("{input_path}.tdf.json"));

        // Read input file
        let plaintext = tokio::fs::read(input_path)
            .await
            .map_err(crate::ToolError::Io)?;

        // Create service and policy
        let service = OpenTdfService::with_kas_url(kas_url);
        let policy = PolicyBuilder::new()
            .attribute(namespace, &values)
            .build()
            .map_err(|e| crate::ToolError::Execution(format!("Policy error: {e}")))?;

        // Encrypt
        let manifest = service
            .encrypt(&plaintext, &policy)
            .await
            .map_err(|e| crate::ToolError::Execution(format!("Encryption failed: {e}")))?;

        // Write output
        let json = serde_json::to_string_pretty(&manifest)?;
        tokio::fs::write(&output_path, &json)
            .await
            .map_err(crate::ToolError::Io)?;

        Ok(json!({
            "success": true,
            "input_path": input_path,
            "output_path": output_path,
            "payload_size": manifest.payload.value.len(),
            "algorithm": manifest.encryption_information.method.algorithm,
            "kas_url": kas_url,
            "namespace": namespace,
            "values": values
        }))
    }
}

/// MCP tool for displaying TDF manifest information.
pub struct TdfInfoTool {
    schema: ToolSchema,
}

impl TdfInfoTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "tdf_info".to_string(),
                aliases: Some(vec!["inspect_tdf".to_string()]),
                description: "Display information about a TDF manifest file. \
                    Shows version, payload details, encryption method, key access, and policy."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "input_path": {
                            "type": "string",
                            "description": "Path to the TDF manifest file (.tdf.json)"
                        }
                    },
                    "required": ["input_path"]
                }),
            },
        }
    }
}

impl Default for TdfInfoTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TdfInfoTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let input_path = args["input_path"]
            .as_str()
            .ok_or_else(|| crate::ToolError::InvalidParams("Missing input_path".to_string()))?;

        // Read and parse manifest
        let json_str = tokio::fs::read_to_string(input_path)
            .await
            .map_err(crate::ToolError::Io)?;

        let manifest: TdfManifest = serde_json::from_str(&json_str)
            .map_err(|e| crate::ToolError::Execution(format!("Invalid TDF manifest: {e}")))?;

        // Build key access info
        let key_access: Vec<Value> = manifest
            .encryption_information
            .key_access
            .iter()
            .map(|ka| {
                json!({
                    "type": ka.access_type,
                    "url": ka.url,
                    "protocol": ka.protocol
                })
            })
            .collect();

        Ok(json!({
            "file": input_path,
            "version": manifest.version,
            "payload": {
                "type": manifest.payload.payload_type,
                "mime_type": manifest.payload.mime_type,
                "protocol": manifest.payload.protocol,
                "size_bytes": manifest.payload.value.len()
            },
            "encryption": {
                "type": manifest.encryption_information.key_type,
                "algorithm": manifest.encryption_information.method.algorithm,
                "streamable": manifest.encryption_information.method.is_streamable
            },
            "key_access": key_access,
            "policy_base64": manifest.encryption_information.policy
        }))
    }
}

/// MCP tool for listing available TDF operations.
pub struct TdfHelpTool {
    schema: ToolSchema,
}

impl TdfHelpTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "tdf_help".to_string(),
                aliases: None,
                description: "List available TDF (Trusted Data Format) operations and usage."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
        }
    }
}

impl Default for TdfHelpTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TdfHelpTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, _args: Value) -> crate::Result<Value> {
        Ok(json!({
            "description": "TDF (Trusted Data Format) provides attribute-based encryption for data protection",
            "tools": [
                {
                    "name": "tdf_encrypt",
                    "description": "Encrypt a file using OpenTDF format with AES-256-GCM",
                    "required_params": ["input_path"],
                    "optional_params": ["output_path", "namespace", "values", "kas_url"]
                },
                {
                    "name": "tdf_info",
                    "description": "Display TDF manifest information",
                    "required_params": ["input_path"]
                }
            ],
            "examples": [
                {
                    "description": "Encrypt a file with default policy",
                    "call": {
                        "tool": "tdf_encrypt",
                        "params": { "input_path": "/path/to/secret.txt" }
                    }
                },
                {
                    "description": "Encrypt with custom attributes",
                    "call": {
                        "tool": "tdf_encrypt",
                        "params": {
                            "input_path": "/path/to/secret.txt",
                            "namespace": "https://arkavo.net/attr/classification",
                            "values": ["top-secret", "nato"]
                        }
                    }
                },
                {
                    "description": "Inspect a TDF file",
                    "call": {
                        "tool": "tdf_info",
                        "params": { "input_path": "/path/to/secret.txt.tdf.json" }
                    }
                }
            ]
        }))
    }
}

// Iroh P2P transport tools (feature-gated)
#[cfg(feature = "iroh")]
mod iroh_tools {
    use crate::server::Tool;
    use arkavo_mcp::ToolSchema;
    use arkavo_tdf_iroh::{IrohTicket, IrohTransport};
    use async_trait::async_trait;
    use serde_json::{Value, json};

    /// MCP tool for staging data to Iroh P2P network.
    pub struct TdfStageTool {
        schema: ToolSchema,
    }

    impl TdfStageTool {
        pub fn new() -> Self {
            Self {
                schema: ToolSchema {
                    name: "tdf_stage".to_string(),
                    aliases: Some(vec!["iroh_stage".to_string()]),
                    description: "Stage a file to the Iroh P2P network. \
                        Returns a ticket that can be used to fetch the data from any Iroh node."
                        .to_string(),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "input_path": {
                                "type": "string",
                                "description": "Path to the file to stage"
                            }
                        },
                        "required": ["input_path"]
                    }),
                },
            }
        }
    }

    impl Default for TdfStageTool {
        fn default() -> Self {
            Self::new()
        }
    }

    #[async_trait]
    impl Tool for TdfStageTool {
        fn schema(&self) -> &ToolSchema {
            &self.schema
        }

        async fn execute(&self, args: Value) -> crate::Result<Value> {
            let input_path = args["input_path"]
                .as_str()
                .ok_or_else(|| crate::ToolError::InvalidParams("Missing input_path".to_string()))?;

            // Read file
            let data = tokio::fs::read(input_path)
                .await
                .map_err(crate::ToolError::Io)?;

            // Create Iroh transport and stage
            let transport = IrohTransport::new().await.map_err(|e| {
                crate::ToolError::Execution(format!("Failed to create transport: {e}"))
            })?;

            let ticket = transport
                .stage_bytes(&data)
                .await
                .map_err(|e| crate::ToolError::Execution(format!("Failed to stage: {e}")))?;

            // Get hash from inner BlobTicket
            let hash = ticket.inner().hash().to_string();

            Ok(json!({
                "success": true,
                "input_path": input_path,
                "size_bytes": data.len(),
                "hash": hash,
                "ticket": ticket.to_string()
            }))
        }
    }

    /// MCP tool for fetching data from Iroh P2P network.
    pub struct TdfFetchTool {
        schema: ToolSchema,
    }

    impl TdfFetchTool {
        pub fn new() -> Self {
            Self {
                schema: ToolSchema {
                    name: "tdf_fetch".to_string(),
                    aliases: Some(vec!["iroh_fetch".to_string()]),
                    description: "Fetch data from the Iroh P2P network using a ticket. \
                        The ticket contains the content hash and node addresses for retrieval."
                        .to_string(),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "ticket": {
                                "type": "string",
                                "description": "Iroh blob ticket for the data to fetch"
                            },
                            "output_path": {
                                "type": "string",
                                "description": "Path to write the fetched data"
                            }
                        },
                        "required": ["ticket", "output_path"]
                    }),
                },
            }
        }
    }

    impl Default for TdfFetchTool {
        fn default() -> Self {
            Self::new()
        }
    }

    #[async_trait]
    impl Tool for TdfFetchTool {
        fn schema(&self) -> &ToolSchema {
            &self.schema
        }

        async fn execute(&self, args: Value) -> crate::Result<Value> {
            let ticket_str = args["ticket"]
                .as_str()
                .ok_or_else(|| crate::ToolError::InvalidParams("Missing ticket".to_string()))?;

            let output_path = args["output_path"].as_str().ok_or_else(|| {
                crate::ToolError::InvalidParams("Missing output_path".to_string())
            })?;

            // Parse ticket
            let ticket: IrohTicket = ticket_str
                .parse()
                .map_err(|e| crate::ToolError::InvalidParams(format!("Invalid ticket: {e}")))?;

            // Create Iroh transport and fetch
            let transport = IrohTransport::new().await.map_err(|e| {
                crate::ToolError::Execution(format!("Failed to create transport: {e}"))
            })?;

            let data = transport
                .fetch_bytes(&ticket)
                .await
                .map_err(|e| crate::ToolError::Execution(format!("Failed to fetch: {e}")))?;

            // Write to output
            tokio::fs::write(output_path, &data)
                .await
                .map_err(crate::ToolError::Io)?;

            // Get hash from inner BlobTicket
            let hash = ticket.inner().hash().to_string();

            Ok(json!({
                "success": true,
                "output_path": output_path,
                "size_bytes": data.len(),
                "hash": hash
            }))
        }
    }
}

#[cfg(feature = "iroh")]
pub use iroh_tools::{TdfFetchTool, TdfStageTool};

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_encrypt_tool_schema() {
        let tool = TdfEncryptTool::new();
        let schema = tool.schema();

        assert_eq!(schema.name, "tdf_encrypt");
        assert!(schema.description.contains("OpenTDF"));
        assert!(
            schema
                .aliases
                .as_ref()
                .unwrap()
                .contains(&"encrypt_tdf".to_string())
        );
    }

    #[tokio::test]
    async fn test_info_tool_schema() {
        let tool = TdfInfoTool::new();
        let schema = tool.schema();

        assert_eq!(schema.name, "tdf_info");
        assert!(schema.description.contains("manifest"));
    }

    #[tokio::test]
    async fn test_help_tool() {
        let tool = TdfHelpTool::new();
        let result = tool.execute(json!({})).await.unwrap();

        assert!(result.get("tools").is_some());
        assert!(result.get("examples").is_some());
    }

    #[tokio::test]
    async fn test_encrypt_and_info_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let input_path = temp_dir.path().join("test.txt");
        let output_path = temp_dir.path().join("test.txt.tdf.json");

        // Create test file
        tokio::fs::write(&input_path, b"Hello, TDF MCP!")
            .await
            .unwrap();

        // Encrypt
        let encrypt_tool = TdfEncryptTool::new();
        let result = encrypt_tool
            .execute(json!({
                "input_path": input_path.to_str().unwrap(),
                "output_path": output_path.to_str().unwrap()
            }))
            .await
            .unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert!(result["payload_size"].as_u64().unwrap() > 0);

        // Info
        let info_tool = TdfInfoTool::new();
        let info = info_tool
            .execute(json!({
                "input_path": output_path.to_str().unwrap()
            }))
            .await
            .unwrap();

        assert_eq!(info["version"], "3.0.0");
        assert_eq!(info["encryption"]["algorithm"], "AES-256-GCM");
    }

    #[tokio::test]
    async fn test_encrypt_missing_input() {
        let tool = TdfEncryptTool::new();
        let result = tool.execute(json!({})).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_info_invalid_file() {
        let tool = TdfInfoTool::new();
        let result = tool
            .execute(json!({
                "input_path": "/nonexistent/file.tdf.json"
            }))
            .await;

        assert!(result.is_err());
    }

    #[cfg(feature = "iroh")]
    mod iroh_tests {
        use super::*;

        #[tokio::test]
        async fn test_stage_tool_schema() {
            let tool = TdfStageTool::new();
            let schema = tool.schema();

            assert_eq!(schema.name, "tdf_stage");
            assert!(schema.description.contains("Iroh"));
            assert!(
                schema
                    .aliases
                    .as_ref()
                    .unwrap()
                    .contains(&"iroh_stage".to_string())
            );
        }

        #[tokio::test]
        async fn test_fetch_tool_schema() {
            let tool = TdfFetchTool::new();
            let schema = tool.schema();

            assert_eq!(schema.name, "tdf_fetch");
            assert!(schema.description.contains("Iroh"));
            assert!(
                schema
                    .aliases
                    .as_ref()
                    .unwrap()
                    .contains(&"iroh_fetch".to_string())
            );
        }

        #[tokio::test]
        async fn test_stage_missing_input() {
            let tool = TdfStageTool::new();
            let result = tool.execute(json!({})).await;

            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_fetch_missing_ticket() {
            let tool = TdfFetchTool::new();
            let result = tool.execute(json!({})).await;

            assert!(result.is_err());
        }
    }
}
