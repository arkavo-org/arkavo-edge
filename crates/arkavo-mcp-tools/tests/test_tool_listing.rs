// Test to list all tools and verify filesystem tool registration
use arkavo_mcp_tools::{ToolRegistry, DetailLevel};

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
    assert!(direct.is_some(), "Should find filesystem_tools by direct name");

    if let Some(tool) = direct {
        let schema = tool.schema();
        println!("  ✓ Found: {}", schema.name);
        assert_eq!(schema.name, "filesystem_tools");

        if let Some(aliases) = &schema.aliases {
            println!("    Aliases: {:?}", aliases);
            assert!(aliases.contains(&"filesystem".to_string()), "Should have 'filesystem' alias");
            assert!(aliases.contains(&"fs".to_string()), "Should have 'fs' alias");
        }
    }

    // Test alias lookups
    println!("\nAlias lookup 'filesystem':");
    let alias1 = registry.get("filesystem");
    assert!(alias1.is_some(), "Should find filesystem_tools via 'filesystem' alias");

    if let Some(tool) = alias1 {
        let schema = tool.schema();
        println!("  ✓ Found via alias: {} (actual name: {})", "filesystem", schema.name);
        assert_eq!(schema.name, "filesystem_tools");
    }

    println!("\nAlias lookup 'fs':");
    let alias2 = registry.get("fs");
    assert!(alias2.is_some(), "Should find filesystem_tools via 'fs' alias");

    if let Some(tool) = alias2 {
        let schema = tool.schema();
        println!("  ✓ Found via alias: {} (actual name: {})", "fs", schema.name);
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
    assert!(has_filesystem, "Search for 'file' should include filesystem_tools");
}

#[tokio::test]
async fn test_total_tool_count() {
    let registry = ToolRegistry::new();

    println!("\n=== Total Tool Count ===");
    let all_tools = registry.search_tools("", DetailLevel::NameOnly);
    println!("Total registered tools: {}", all_tools.len());

    assert!(all_tools.len() > 0, "Should have at least one registered tool");

    // Verify filesystem_tools is in the registry
    let has_filesystem = all_tools.iter().any(|t| t.name == "filesystem_tools");
    assert!(has_filesystem, "Registry should include filesystem_tools");
}
