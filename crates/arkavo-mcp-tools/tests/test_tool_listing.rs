// Test to list all tools and verify filesystem tool registration
use arkavo_mcp_tools::{DetailLevel, ToolRegistry};

#[tokio::test]
async fn test_list_all_tools() {
    let registry = ToolRegistry::new();

    println!("=== All Registered Tools ===\n");

    // Test the search_tools functionality with different detail levels
    println!("1. NAME_ONLY level:");
    let name_only = registry.search_tools("", DetailLevel::NameOnly);
    for tool in &name_only {
        println!("  - {}", tool.name);
    }
    assert!(!name_only.is_empty(), "Should have registered tools");

    println!("\n2. NAME_AND_DESCRIPTION level:");
    let with_desc = registry.search_tools("", DetailLevel::NameAndDescription);
    for tool in &with_desc {
        if let Some(desc) = &tool.description {
            println!("  - {}: {}", tool.name, desc);
        }
    }

    println!("\n3. FULL_SCHEMA level (with schemas):");
    let full = registry.search_tools("", DetailLevel::FullSchema);
    for tool in &full {
        println!("\n  Tool: {}", tool.name);
        if let Some(desc) = &tool.description {
            println!("    Description: {}", desc);
        }
        if let Some(schema) = &tool.schema {
            println!("    Schema: {} bytes", schema.to_string().len());
        }
    }

    // Check aliases by directly accessing tool schemas
    println!("\n4. Tools with aliases:");
    for tool_name in &["filesystem_tools", "git_status", "git_diff", "git_commit"] {
        if let Some(tool) = registry.get(tool_name) {
            let schema = tool.schema();
            if let Some(aliases) = &schema.aliases {
                if !aliases.is_empty() {
                    println!("  - {}: {:?}", schema.name, aliases);
                }
            }
        }
    }
}

#[tokio::test]
async fn test_filesystem_tool_registration() {
    let registry = ToolRegistry::new();

    println!("\n=== Filesystem Tool Verification ===\n");

    // Test direct lookup
    println!("Direct lookup 'filesystem_tools':");
    let direct = registry.get("filesystem_tools");
    assert!(
        direct.is_some(),
        "Should find filesystem_tools by direct name"
    );

    if let Some(tool) = direct {
        let schema = tool.schema();
        println!("  ✓ Found: {}", schema.name);
        assert_eq!(schema.name, "filesystem_tools");

        if let Some(aliases) = &schema.aliases {
            println!("    Aliases: {:?}", aliases);
            assert!(
                aliases.contains(&"filesystem".to_string()),
                "Should have 'filesystem' alias"
            );
            assert!(
                aliases.contains(&"fs".to_string()),
                "Should have 'fs' alias"
            );
        }
    }

    // Test alias lookups
    println!("\nAlias lookup 'filesystem':");
    let alias1 = registry.get("filesystem");
    assert!(
        alias1.is_some(),
        "Should find filesystem_tools via 'filesystem' alias"
    );

    if let Some(tool) = alias1 {
        let schema = tool.schema();
        println!(
            "  ✓ Found via alias: {} (actual name: {})",
            "filesystem", schema.name
        );
        assert_eq!(schema.name, "filesystem_tools");
    }

    println!("\nAlias lookup 'fs':");
    let alias2 = registry.get("fs");
    assert!(
        alias2.is_some(),
        "Should find filesystem_tools via 'fs' alias"
    );

    if let Some(tool) = alias2 {
        let schema = tool.schema();
        println!(
            "  ✓ Found via alias: {} (actual name: {})",
            "fs", schema.name
        );
        assert_eq!(schema.name, "filesystem_tools");
    }
}

#[tokio::test]
async fn test_search_file_tools() {
    let registry = ToolRegistry::new();

    println!("\n=== Search for 'file' ===");
    let file_tools = registry.search_tools("file", DetailLevel::NameAndDescription);

    assert!(!file_tools.is_empty(), "Should find file-related tools");

    for tool in &file_tools {
        println!("  - {}", tool.name);
        if let Some(desc) = &tool.description {
            println!("    {}", desc);
        }
    }

    // Verify filesystem_tools is in the search results
    let has_filesystem = file_tools.iter().any(|t| t.name == "filesystem_tools");
    assert!(
        has_filesystem,
        "Search for 'file' should include filesystem_tools"
    );
}

#[tokio::test]
async fn test_total_tool_count() {
    let registry = ToolRegistry::new();

    println!("\n=== Total Tool Count ===");
    let all_tools = registry.search_tools("", DetailLevel::NameOnly);
    println!("Total registered tools: {}", all_tools.len());

    assert!(
        all_tools.len() > 0,
        "Should have at least one registered tool"
    );

    // Verify filesystem_tools is in the registry
    let has_filesystem = all_tools.iter().any(|t| t.name == "filesystem_tools");
    assert!(has_filesystem, "Registry should include filesystem_tools");
}

#[tokio::test]
async fn test_time_tool_discovery_by_alias() {
    let registry = ToolRegistry::new();

    println!("\n=== Time Tool Discovery (Regression Test) ===");

    // Test that searching for "time" finds the time tool
    let time_tools = registry.search_tools("time", DetailLevel::FullSchema);

    println!("Search for 'time' found {} tools:", time_tools.len());
    for tool in &time_tools {
        println!("  - {}", tool.name);
        if let Some(aliases) = &tool.aliases {
            println!("    Aliases: {:?}", aliases);
        }
    }

    // Should find get_agent_time (which has "time" alias)
    let has_time_tool = time_tools.iter().any(|t| t.name == "get_agent_time");
    assert!(
        has_time_tool,
        "Search for 'time' should find get_agent_time tool"
    );

    // Verify the tool has the expected aliases
    if let Some(time_tool) = time_tools.iter().find(|t| t.name == "get_agent_time") {
        assert!(
            time_tool.aliases.is_some(),
            "get_agent_time should have aliases"
        );

        if let Some(aliases) = &time_tool.aliases {
            assert!(
                aliases.contains(&"time".to_string()),
                "get_agent_time should have 'time' alias"
            );
            println!("  ✓ Confirmed 'time' alias exists");

            // Verify aliases are displayed in FullSchema detail level
            println!("  ✓ Aliases visible in FullSchema: {:?}", aliases);
        }
    }

    // Test other time-related keywords
    println!("\nSearch for 'clock':");
    let clock_tools = registry.search_tools("clock", DetailLevel::FullSchema);
    let has_clock = clock_tools.iter().any(|t| t.name == "get_agent_time");
    assert!(
        has_clock,
        "Search for 'clock' should also find get_agent_time"
    );
    println!("  ✓ Found {} tools", clock_tools.len());

    println!("\nSearch for 'date':");
    let date_tools = registry.search_tools("date", DetailLevel::FullSchema);
    let has_date = date_tools.iter().any(|t| t.name == "get_agent_time");
    assert!(
        has_date,
        "Search for 'date' should also find get_agent_time"
    );
    println!("  ✓ Found {} tools", date_tools.len());
}

#[tokio::test]
async fn test_judge_sees_time_tool() {
    let registry = ToolRegistry::new();

    println!("\n=== Judge Tool Visibility Test ===");

    // Get all tools (simulating what the judge sees)
    let all_tools = registry.search_tools("", DetailLevel::NameAndDescription);

    println!("Total tools visible to judge: {}", all_tools.len());

    // Find get_agent_time in the list
    let time_tool_position = all_tools
        .iter()
        .position(|t| t.name == "get_agent_time")
        .expect("get_agent_time should be in the registry");

    println!("get_agent_time is at position: {}", time_tool_position + 1);

    // Judge now shows first 25 tools (was 10 before fix)
    assert!(
        time_tool_position < 25,
        "get_agent_time should be in first 25 tools (visible to judge). Position: {}",
        time_tool_position + 1
    );

    println!(
        "  ✓ get_agent_time is at position {}, within judge's visibility (25 tools)",
        time_tool_position + 1
    );

    // Print the first 25 tools to verify
    println!("\nFirst 25 tools (what judge sees):");
    for (i, tool) in all_tools.iter().take(25).enumerate() {
        println!("  {}. {}", i + 1, tool.name);
    }
}

#[tokio::test]
async fn test_mcp_includes_native_tools() {
    use arkavo_mcp_tools::{McpClient, McpTool, ToolRegistry};
    use serde_json::Value;
    use std::sync::Arc;

    // Mock MCP client that returns a few tools
    struct MockMcpClient;

    impl McpClient for MockMcpClient {
        fn list_tools(&self) -> Result<Vec<McpTool>, Box<dyn std::error::Error>> {
            Ok(vec![McpTool {
                name: "mcp_test_tool".to_string(),
                description: "A test tool from MCP".to_string(),
                input_schema: Some(serde_json::json!({
                    "type": "object",
                    "properties": {}
                })),
            }])
        }

        fn call_tool(
            &self,
            _tool_name: &str,
            _args: Value,
            _llm_origin: &str,
        ) -> Result<Value, Box<dyn std::error::Error>> {
            Ok(serde_json::json!({"result": "ok"}))
        }
    }

    println!("\n=== MCP Registry Includes Native Tools Test ===");

    let mcp_client = Arc::new(MockMcpClient);
    let registry =
        ToolRegistry::from_mcp_connection(mcp_client).expect("Should create registry from MCP");

    let all_tools = registry.list_tools();
    println!("Total tools in MCP registry: {}", all_tools.len());

    // Should have native tools + MCP tools
    let has_time_tool = all_tools.iter().any(|t| t.name == "get_agent_time");
    assert!(
        has_time_tool,
        "MCP registry should include native time tools"
    );

    let has_filesystem = all_tools.iter().any(|t| t.name == "filesystem_tools");
    assert!(
        has_filesystem,
        "MCP registry should include native filesystem tools"
    );

    let has_mcp_tool = all_tools.iter().any(|t| t.name == "mcp_test_tool");
    assert!(has_mcp_tool, "MCP registry should include MCP tools");

    println!("  ✓ Native tools present: filesystem, time, etc.");
    println!("  ✓ MCP tools present: mcp_test_tool");
    println!("  ✓ Total: {} tools", all_tools.len());
}
