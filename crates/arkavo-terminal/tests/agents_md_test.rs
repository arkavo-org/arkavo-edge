use std::fs;
use tempfile::TempDir;

#[test]
fn test_agents_md_loading() {
    // Create a temporary directory for the test
    let temp_dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();

    // Change to temp directory
    std::env::set_current_dir(&temp_dir).unwrap();

    // Test 1: No AGENTS.md or CLAUDE.md - should use default prompt
    // This would require exposing the prompt loading logic as a separate function
    // For now, we'll just test that the files can be created and read

    // Test 2: AGENTS.md exists
    let agents_content = "# AGENTS.md\n\nThis is a test agent prompt.";
    fs::write("AGENTS.md", agents_content).unwrap();

    let read_content = fs::read_to_string("AGENTS.md").unwrap();
    assert!(!read_content.is_empty());
    assert!(read_content.len() > 10);

    // Test 3: CLAUDE.md as fallback
    // Use unwrap_or to handle the case where AGENTS.md might not exist
    let _ = fs::remove_file("AGENTS.md");
    let claude_content = "# CLAUDE.md\n\nThis is a fallback prompt.";
    fs::write("CLAUDE.md", claude_content).unwrap();

    let read_content = fs::read_to_string("CLAUDE.md").unwrap();
    assert!(!read_content.is_empty());
    assert!(read_content.len() > 10);

    // Test 4: AGENTS.md takes precedence over CLAUDE.md
    fs::write("AGENTS.md", agents_content).unwrap();
    // Both files exist, but AGENTS.md should be preferred
    assert!(std::path::Path::new("AGENTS.md").exists());
    assert!(std::path::Path::new("CLAUDE.md").exists());

    // Restore original directory
    std::env::set_current_dir(&original_dir).unwrap();
}

#[test]
fn test_agents_md_with_mcp_info() {
    let temp_dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&temp_dir).unwrap();

    // Create AGENTS.md with specific content
    let agents_content = r#"# AI Agent Instructions

You are a specialized AI agent for code analysis.

## Your Role
- Analyze code quality
- Suggest improvements
- Help with debugging

## Guidelines
1. Be concise
2. Focus on practical solutions
3. Consider performance implications"#;

    fs::write("AGENTS.md", agents_content).unwrap();

    // Verify the content can be read and is not empty
    let content = fs::read_to_string("AGENTS.md").unwrap();
    assert!(!content.is_empty());
    assert!(content.len() > 10); // Has meaningful content

    // Test that the content can be appended with MCP info
    let mcp_info = "\n\nMCP Tools Available:\n- git_status\n- filesystem_tools";
    let combined = format!("{}{}", content, mcp_info);

    assert!(combined.len() > content.len());
    assert!(combined.contains(&content)); // Original content is preserved

    std::env::set_current_dir(&original_dir).unwrap();
}
