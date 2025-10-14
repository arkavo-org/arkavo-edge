#![allow(clippy::disallowed_methods)]
#![cfg(target_os = "macos")]

use arkavo_mcp_macos::mcp::filesystem_tools::FileSystemKit;
use arkavo_mcp_macos::mcp::server::Tool;
use serde_json::json;
use tempfile::TempDir;

#[tokio::test]
async fn test_filesystem_kit_read_agents_md() {
    let temp_dir = TempDir::new().unwrap();

    // Create AGENTS.md with absolute path
    let agents_path = temp_dir.path().join("AGENTS.md");
    let agents_content = r#"# AGENTS.md

This is the system prompt for the AI agent.

## Instructions
- Follow these guidelines
- Be helpful and accurate"#;

    std::fs::write(&agents_path, agents_content).unwrap();

    // Test reading AGENTS.md using FileSystemKit
    let kit = FileSystemKit::new();
    let params = json!({
        "action": "read_file",
        "file_path": agents_path.to_str().unwrap()
    });

    let result = kit.execute(params).await.unwrap();
    assert_eq!(result["success"], true);
    assert!(!result["content"].as_str().unwrap().is_empty());
    // Just verify it has some content
    let content = result["content"].as_str().unwrap();
    assert!(content.len() > 10);
}

#[tokio::test]
async fn test_filesystem_kit_list_directory_with_agents() {
    let temp_dir = TempDir::new().unwrap();

    // Create files with absolute paths
    std::fs::write(temp_dir.path().join("AGENTS.md"), "Agent content").unwrap();
    std::fs::write(temp_dir.path().join("CLAUDE.md"), "Claude content").unwrap();
    std::fs::write(temp_dir.path().join("README.md"), "Readme content").unwrap();

    // List directory using absolute path
    let kit = FileSystemKit::new();
    let params = json!({
        "action": "list_directory",
        "dir_path": temp_dir.path().to_str().unwrap()
    });

    let result = kit.execute(params).await.unwrap();
    assert_eq!(result["success"], true);

    let entries = result["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3); // Exactly our 3 files in a temp directory

    // Check that all files are present
    let file_names: Vec<String> = entries
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect();

    assert!(file_names.contains(&"AGENTS.md".to_string()));
    assert!(file_names.contains(&"CLAUDE.md".to_string()));
    assert!(file_names.contains(&"README.md".to_string()));
}

#[tokio::test]
async fn test_filesystem_kit_file_info_agents() {
    let temp_dir = TempDir::new().unwrap();

    // Create AGENTS.md with specific content
    let agents_path = temp_dir.path().join("AGENTS.md");
    let content = "This is AGENTS.md content for testing";
    std::fs::write(&agents_path, content).unwrap();

    // Get file info
    let kit = FileSystemKit::new();
    let params = json!({
        "action": "file_info",
        "file_path": agents_path.to_str().unwrap()
    });

    let result = kit.execute(params).await.unwrap();
    assert_eq!(result["success"], true);
    assert_eq!(result["is_file"], true);
    assert_eq!(result["is_directory"], false);
    assert_eq!(result["size"], content.len() as u64);
}
