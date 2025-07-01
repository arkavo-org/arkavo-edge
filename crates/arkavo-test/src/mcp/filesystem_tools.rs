use async_trait::async_trait;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

use crate::mcp::server::{Tool, ToolSchema};
use crate::{Result, TestError};

pub struct FileSystemKit {
    schema: ToolSchema,
}

impl FileSystemKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "filesystem_tools".to_string(),
                description: "Tools for reading files and directories".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["read_file", "list_directory", "file_info"],
                            "description": "The action to perform"
                        },
                        "file_path": {
                            "type": "string",
                            "description": "Path to the file (for read_file and file_info)"
                        },
                        "dir_path": {
                            "type": "string",
                            "description": "Path to the directory (for list_directory)"
                        }
                    },
                    "required": ["action"]
                }),
            },
        }
    }

    fn validate_path(&self, path: &str) -> Result<PathBuf> {
        let path = Path::new(path);

        // Convert to absolute path if relative
        let abs_path = if path.is_relative() {
            std::env::current_dir()
                .map_err(|e| TestError::Mcp(format!("Failed to get current directory: {e}")))?
                .join(path)
        } else {
            path.to_path_buf()
        };

        // Basic security check - ensure path doesn't contain suspicious patterns
        let path_str = abs_path.to_string_lossy();
        if path_str.contains("..") || path_str.contains('~') {
            return Err(TestError::Mcp("Path traversal not allowed".to_string()));
        }

        Ok(abs_path)
    }
}

impl Default for FileSystemKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for FileSystemKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params["action"]
            .as_str()
            .ok_or_else(|| TestError::Mcp("Missing 'action' parameter".to_string()))?;

        match action {
            "read_file" => {
                let file_path = params["file_path"]
                    .as_str()
                    .ok_or_else(|| TestError::Mcp("Missing 'file_path' parameter".to_string()))?;

                let abs_path = self.validate_path(file_path)?;

                // Check if file exists
                if !abs_path.exists() {
                    return Ok(json!({
                        "success": false,
                        "error": format!("File not found: {}", abs_path.display())
                    }));
                }

                // Check if it's a file (not directory)
                if !abs_path.is_file() {
                    return Ok(json!({
                        "success": false,
                        "error": format!("Path is not a file: {}", abs_path.display())
                    }));
                }

                // Read file with size limit (10MB)
                let metadata = fs::metadata(&abs_path)
                    .map_err(|e| TestError::Mcp(format!("Failed to get file metadata: {e}")))?;

                if metadata.len() > 10_485_760 {
                    return Ok(json!({
                        "success": false,
                        "error": "File too large (>10MB)"
                    }));
                }

                let content = fs::read_to_string(&abs_path)
                    .map_err(|e| TestError::Mcp(format!("Failed to read file: {e}")))?;

                Ok(json!({
                    "success": true,
                    "content": content,
                    "path": abs_path.to_string_lossy()
                }))
            }

            "list_directory" => {
                let dir_path = params["dir_path"]
                    .as_str()
                    .ok_or_else(|| TestError::Mcp("Missing 'dir_path' parameter".to_string()))?;

                let abs_path = self.validate_path(dir_path)?;

                // Check if directory exists
                if !abs_path.exists() {
                    return Ok(json!({
                        "success": false,
                        "error": format!("Directory not found: {}", abs_path.display())
                    }));
                }

                // Check if it's a directory
                if !abs_path.is_dir() {
                    return Ok(json!({
                        "success": false,
                        "error": format!("Path is not a directory: {}", abs_path.display())
                    }));
                }

                let mut entries = Vec::new();
                let read_dir = fs::read_dir(&abs_path)
                    .map_err(|e| TestError::Mcp(format!("Failed to read directory: {e}")))?;

                for entry in read_dir {
                    if let Ok(entry) = entry {
                        let file_type = entry
                            .file_type()
                            .map_err(|e| TestError::Mcp(format!("Failed to get file type: {e}")))?;

                        entries.push(json!({
                            "name": entry.file_name().to_string_lossy(),
                            "type": if file_type.is_dir() { "directory" } else { "file" }
                        }));
                    }
                }

                Ok(json!({
                    "success": true,
                    "path": abs_path.to_string_lossy(),
                    "entries": entries
                }))
            }

            "file_info" => {
                let file_path = params["file_path"]
                    .as_str()
                    .ok_or_else(|| TestError::Mcp("Missing 'file_path' parameter".to_string()))?;

                let abs_path = self.validate_path(file_path)?;

                // Check if path exists
                if !abs_path.exists() {
                    return Ok(json!({
                        "success": false,
                        "error": format!("Path not found: {}", abs_path.display())
                    }));
                }

                let metadata = fs::metadata(&abs_path)
                    .map_err(|e| TestError::Mcp(format!("Failed to get metadata: {e}")))?;

                Ok(json!({
                    "success": true,
                    "path": abs_path.to_string_lossy(),
                    "exists": true,
                    "is_file": metadata.is_file(),
                    "is_directory": metadata.is_dir(),
                    "size": metadata.len(),
                    "readonly": metadata.permissions().readonly()
                }))
            }

            _ => Err(TestError::Mcp(format!("Unknown action: {action}"))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio;

    #[tokio::test]
    async fn test_read_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, "Hello, World!").unwrap();

        let kit = FileSystemKit::new();
        let params = json!({
            "action": "read_file",
            "file_path": file_path.to_str().unwrap()
        });

        let result = kit.execute(params).await.unwrap();
        assert_eq!(result["success"], true);
        assert_eq!(result["content"], "Hello, World!");
    }

    #[tokio::test]
    async fn test_read_nonexistent_file() {
        let kit = FileSystemKit::new();
        let params = json!({
            "action": "read_file",
            "file_path": "/nonexistent/file.txt"
        });

        let result = kit.execute(params).await.unwrap();
        assert_eq!(result["success"], false);
        assert!(result["error"].as_str().unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn test_list_directory() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("file1.txt"), "content1").unwrap();
        fs::write(temp_dir.path().join("file2.txt"), "content2").unwrap();
        fs::create_dir(temp_dir.path().join("subdir")).unwrap();

        let kit = FileSystemKit::new();
        let params = json!({
            "action": "list_directory",
            "dir_path": temp_dir.path().to_str().unwrap()
        });

        let result = kit.execute(params).await.unwrap();
        assert_eq!(result["success"], true);

        let entries = result["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[tokio::test]
    async fn test_file_info() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, "Hello").unwrap();

        let kit = FileSystemKit::new();
        let params = json!({
            "action": "file_info",
            "file_path": file_path.to_str().unwrap()
        });

        let result = kit.execute(params).await.unwrap();
        assert_eq!(result["success"], true);
        assert_eq!(result["is_file"], true);
        assert_eq!(result["is_directory"], false);
        assert_eq!(result["size"], 5);
    }
}
