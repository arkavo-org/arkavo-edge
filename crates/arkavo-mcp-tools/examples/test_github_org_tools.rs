use arkavo_mcp_tools::registry::ToolRegistry;
use arkavo_mcp_tools::Tool;
use arkavo_memory::MemoryStorage;
use serde_json::json;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing GitHub Org Knowledge Tools\n");
    println!("===================================\n");

    let storage = Arc::new(MemoryStorage::new().await?);
    let registry = ToolRegistry::new(storage);
    let all_tools = registry.list_tools();

    println!("Total tools registered: {}\n", all_tools.len());

    let github_tools: Vec<_> = all_tools
        .iter()
        .filter(|t| t.category == "GitHub")
        .collect();

    println!("GitHub category tools: {}\n", github_tools.len());

    for tool in &github_tools {
        println!("Tool: {}", tool.name);
        println!("  Description: {}", tool.description);
        println!("  Category: {}", tool.category);
        println!();
    }

    println!("\nTesting github_org_repos...");
    if let Some(tool) = registry.get("github_org_repos") {
        let params = json!({
            "org": "arkavo-org",
            "limit": 5
        });

        match tool.execute(params).await {
            Ok(result) => {
                println!("✅ Success!");
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            Err(e) => {
                println!("❌ Error: {}", e);
                println!("Note: This requires 'gh' CLI to be installed and authenticated");
            }
        }
    }

    println!("\nTesting github_org_overview...");
    if let Some(tool) = registry.get("github_org_overview") {
        let params = json!({
            "org": "arkavo-org"
        });

        match tool.execute(params).await {
            Ok(result) => {
                println!("✅ Success!");
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            Err(e) => {
                println!("❌ Error: {}", e);
                println!("Note: This requires 'gh' CLI to be installed and authenticated");
            }
        }
    }

    println!("\n===================================");
    println!("Test complete!");
    println!("\nTo use these tools in the CEF UI:");
    println!("1. Run: cargo run -p arkavo -- ui");
    println!("2. Tools will appear in the 'GitHub' category");
    println!("3. Select a tool and provide the required parameters");
    println!("4. Make sure 'gh' CLI is installed: brew install gh");
    println!("5. Authenticate: gh auth login");

    Ok(())
}
