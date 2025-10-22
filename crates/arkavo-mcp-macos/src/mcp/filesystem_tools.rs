use async_trait::async_trait;
use serde_json::{json, Value};
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
                description: "Tools for reading, writing, and editing files".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["read_file", "list_directory", "file_info", "write_file", "edit_file", "append_file"],
                            "description": "The action to perform"
                        },
                        "file_path": {
                            "type": "string",
                            "description": "Path to the file"
                        },
                        "dir_path": {
                            "type": "string",
                            "description": "Path to the directory (for list_directory)"
                        },
                        "content": {
                            "type": "string",
                            "description": "Content to write (for write_file and append_file)"
                        },
                        "line_number": {
                            "type": "integer",
                            "description": "Line number to edit (1-based, for edit_file)"
                        },
                        "new_content": {
                            "type": "string",
                            "description": "New content for the line (for edit_file)"
                        },
                        "mode": {
                            "type": "string",
                            "enum": ["replace", "insert", "delete"],
                            "description": "Edit mode (for edit_file)"
                        },
                        "overwrite": {
                            "type": "boolean",
                            "description": "Whether to overwrite existing file (for write_file, default: true)"
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

                for entry in read_dir.flatten() {
                    let file_type = entry
                        .file_type()
                        .map_err(|e| TestError::Mcp(format!("Failed to get file type: {e}")))?;

                    entries.push(json!({
                        "name": entry.file_name().to_string_lossy(),
                        "type": if file_type.is_dir() { "directory" } else { "file" }
                    }));
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

            "write_file" => {
                let file_path = params["file_path"]
                    .as_str()
                    .ok_or_else(|| TestError::Mcp("Missing 'file_path' parameter".to_string()))?;

                let content = params["content"]
                    .as_str()
                    .ok_or_else(|| TestError::Mcp("Missing 'content' parameter".to_string()))?;

                let overwrite = params["overwrite"].as_bool().unwrap_or(true);

                let abs_path = self.validate_path(file_path)?;

                // Check if file exists and overwrite is false
                if abs_path.exists() && !overwrite {
                    return Ok(json!({
                        "success": false,
                        "error": "File already exists and overwrite is disabled"
                    }));
                }

                // Create parent directories if they don't exist
                if let Some(parent) = abs_path.parent() {
                    fs::create_dir_all(parent).map_err(|e| {
                        TestError::Mcp(format!("Failed to create parent directories: {e}"))
                    })?;
                }

                // Write file with content
                fs::write(&abs_path, content)
                    .map_err(|e| TestError::Mcp(format!("Failed to write file: {e}")))?;

                Ok(json!({
                    "success": true,
                    "path": abs_path.to_string_lossy(),
                    "action": "created"
                }))
            }

            "append_file" => {
                let file_path = params["file_path"]
                    .as_str()
                    .ok_or_else(|| TestError::Mcp("Missing 'file_path' parameter".to_string()))?;

                let content = params["content"]
                    .as_str()
                    .ok_or_else(|| TestError::Mcp("Missing 'content' parameter".to_string()))?;

                let abs_path = self.validate_path(file_path)?;

                // Check if file exists
                if !abs_path.exists() {
                    return Ok(json!({
                        "success": false,
                        "error": "File does not exist - use write_file instead"
                    }));
                }

                // Append content to file
                let mut file = fs::OpenOptions::new()
                    .append(true)
                    .open(&abs_path)
                    .map_err(|e| {
                        TestError::Mcp(format!("Failed to open file for appending: {e}"))
                    })?;

                use std::io::Write;
                writeln!(file, "{content}")
                    .map_err(|e| TestError::Mcp(format!("Failed to append to file: {e}")))?;

                Ok(json!({
                    "success": true,
                    "path": abs_path.to_string_lossy(),
                    "action": "appended"
                }))
            }

            "edit_file" => {
                let file_path = params["file_path"]
                    .as_str()
                    .ok_or_else(|| TestError::Mcp("Missing 'file_path' parameter".to_string()))?;

                let line_number = params["line_number"]
                    .as_u64()
                    .ok_or_else(|| TestError::Mcp("Missing 'line_number' parameter".to_string()))?
                    as usize;

                let new_content = params["new_content"]
                    .as_str()
                    .ok_or_else(|| TestError::Mcp("Missing 'new_content' parameter".to_string()))?;

                let mode = params["mode"].as_str().unwrap_or("replace");

                let abs_path = self.validate_path(file_path)?;

                // Check if file exists
                if !abs_path.exists() {
                    return Ok(json!({
                        "success": false,
                        "error": "File does not exist"
                    }));
                }

                // Read current file content
                let current_content = fs::read_to_string(&abs_path)
                    .map_err(|e| TestError::Mcp(format!("Failed to read file: {e}")))?;

                let mut lines: Vec<String> = current_content.lines().map(String::from).collect();

                // Validate line number
                if line_number < 1 || line_number > lines.len() + 1 {
                    return Ok(json!({
                        "success": false,
                        "error": format!("Line number {} is out of range (1-{})", line_number, lines.len() + 1)
                    }));
                }

                // Perform the edit operation
                match mode {
                    "replace" => {
                        if line_number > lines.len() {
                            lines.push(new_content.to_string());
                        } else {
                            lines[line_number - 1] = new_content.to_string();
                        }
                    }
                    "insert" => {
                        if line_number > lines.len() + 1 {
                            return Ok(json!({
                                "success": false,
                                "error": format!("Line number {} is out of range for insert (1-{})", line_number, lines.len() + 1)
                            }));
                        }
                        lines.insert(line_number - 1, new_content.to_string());
                    }
                    "delete" => {
                        if line_number > lines.len() {
                            return Ok(json!({
                                "success": false,
                                "error": format!("Line number {} is out of range for delete (1-{})", line_number, lines.len())
                            }));
                        }
                        lines.remove(line_number - 1);
                    }
                    _ => {
                        return Ok(json!({
                            "success": false,
                            "error": format!("Unknown edit mode: {}", mode)
                        }));
                    }
                }

                // Write modified content back to file
                let new_content = lines.join("\n");
                fs::write(&abs_path, new_content)
                    .map_err(|e| TestError::Mcp(format!("Failed to write modified file: {e}")))?;

                Ok(json!({
                    "success": true,
                    "path": abs_path.to_string_lossy(),
                    "action": mode,
                    "line_number": line_number
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
#[allow(clippy::disallowed_methods)]
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

    #[tokio::test]
    async fn test_write_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("new_file.txt");

        let kit = FileSystemKit::new();
        let params = json!({
            "action": "write_file",
            "file_path": file_path.to_str().unwrap(),
            "content": "Hello, World!"
        });

        let result = kit.execute(params).await.unwrap();
        assert_eq!(result["success"], true);
        assert_eq!(result["action"], "created");

        // Verify file was created with correct content
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Hello, World!");
    }

    #[tokio::test]
    async fn test_write_file_no_overwrite() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("existing.txt");
        fs::write(&file_path, "Original content").unwrap();

        let kit = FileSystemKit::new();
        let params = json!({
            "action": "write_file",
            "file_path": file_path.to_str().unwrap(),
            "content": "New content",
            "overwrite": false
        });

        let result = kit.execute(params).await.unwrap();
        assert_eq!(result["success"], false);
        assert!(result["error"].as_str().unwrap().contains("already exists"));

        // Verify original content was preserved
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Original content");
    }

    #[tokio::test]
    async fn test_append_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("append_test.txt");
        fs::write(&file_path, "Line 1\n").unwrap();

        let kit = FileSystemKit::new();
        let params = json!({
            "action": "append_file",
            "file_path": file_path.to_str().unwrap(),
            "content": "Line 2"
        });

        let result = kit.execute(params).await.unwrap();
        assert_eq!(result["success"], true);
        assert_eq!(result["action"], "appended");

        // Verify content was appended
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Line 1\nLine 2\n");
    }

    #[tokio::test]
    async fn test_edit_file_replace() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("edit_test.txt");
        fs::write(&file_path, "Line 1\nLine 2\nLine 3").unwrap();

        let kit = FileSystemKit::new();
        let params = json!({
            "action": "edit_file",
            "file_path": file_path.to_str().unwrap(),
            "line_number": 2,
            "new_content": "Modified Line 2",
            "mode": "replace"
        });

        let result = kit.execute(params).await.unwrap();
        assert_eq!(result["success"], true);
        assert_eq!(result["action"], "replace");

        // Verify line was replaced
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Line 1\nModified Line 2\nLine 3");
    }

    #[tokio::test]
    async fn test_edit_file_insert() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("insert_test.txt");
        fs::write(&file_path, "Line 1\nLine 3").unwrap();

        let kit = FileSystemKit::new();
        let params = json!({
            "action": "edit_file",
            "file_path": file_path.to_str().unwrap(),
            "line_number": 2,
            "new_content": "Line 2",
            "mode": "insert"
        });

        let result = kit.execute(params).await.unwrap();
        assert_eq!(result["success"], true);
        assert_eq!(result["action"], "insert");

        // Verify line was inserted
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Line 1\nLine 2\nLine 3");
    }

    #[tokio::test]
    async fn test_edit_file_delete() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("delete_test.txt");
        fs::write(&file_path, "Line 1\nLine 2\nLine 3").unwrap();

        let kit = FileSystemKit::new();
        let params = json!({
            "action": "edit_file",
            "file_path": file_path.to_str().unwrap(),
            "line_number": 2,
            "new_content": "",
            "mode": "delete"
        });

        let result = kit.execute(params).await.unwrap();
        assert_eq!(result["success"], true);
        assert_eq!(result["action"], "delete");

        // Verify line was deleted
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Line 1\nLine 3");
    }

    #[tokio::test]
    async fn test_edit_file_out_of_range() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("range_test.txt");
        fs::write(&file_path, "Line 1\nLine 2").unwrap();

        let kit = FileSystemKit::new();
        let params = json!({
            "action": "edit_file",
            "file_path": file_path.to_str().unwrap(),
            "line_number": 5,
            "new_content": "New line",
            "mode": "replace"
        });

        let result = kit.execute(params).await.unwrap();
        assert_eq!(result["success"], false);
        assert!(result["error"].as_str().unwrap().contains("out of range"));
    }
}
