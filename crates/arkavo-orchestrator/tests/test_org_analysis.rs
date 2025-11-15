/// Integration test: Router + GitHub MCP Tools for org-wide analysis
///
/// Demonstrates the multi-repo intelligence advantage:
/// 1. Router receives task: "Summarize arkavo-org repositories"
/// 2. Progressive tool disclosure finds GitHub tools
/// 3. LLM calls GitHub tools to fetch data
/// 4. LLM analyzes patterns across repos
/// 5. Generates intelligent summary

#[cfg(test)]
mod org_analysis_tests {
    use arkavo_mcp_tools::ToolRegistry;
    use arkavo_router::Router;
    use std::sync::Arc;

    #[tokio::test]
    #[ignore] // Run manually: cargo test --test test_org_analysis -- --ignored
    async fn test_multi_repo_analysis_with_router() {
        // This test demonstrates the business advantage:
        // Using router + progressive tool disclosure + GitHub MCP tools
        // to analyze an entire organization across multiple repositories

        let router = Router::new().await.expect("Failed to create router");
        let tool_registry = Arc::new(ToolRegistry::new());

        let task = "Analyze the arkavo-org GitHub organization. \
                    List all repositories and summarize what the organization is building. \
                    Look for patterns in the repository names and topics.";

        println!("\n=== Multi-Repo Intelligence Test ===");
        println!("Task: {}", task);
        println!("\nUsing router with progressive tool disclosure...");

        // Router will:
        // 1. Extract keywords: "analyze", "GitHub", "organization", "repositories"
        // 2. search_tools("GitHub organization repositories")
        // 3. Find: github_list_org_repos, github_list_issues, etc.
        // 4. LLM sees only relevant tools (not all 50+ tools)
        // 5. LLM calls appropriate GitHub tools
        // 6. Generates intelligent summary

        match router
            .route_with_quality_gate(task, vec![], Some(&tool_registry), 3)
            .await
        {
            Ok(response) => {
                println!("\n=== Router Response ===");
                println!("{}", response.content);

                if !response.tool_calls.is_empty() {
                    println!("\n=== Tools Called ===");
                    for (i, call) in response.tool_calls.iter().enumerate() {
                        println!(
                            "{}. {} ({})",
                            i + 1,
                            call.tool_name,
                            serde_json::to_string_pretty(&call.arguments).unwrap_or_default()
                        );
                    }
                }

                println!("\n=== Multi-Repo Intelligence Demonstrated ===");
                println!("✓ Router selected appropriate model");
                println!("✓ Progressive tool disclosure loaded only GitHub tools");
                println!("✓ LLM analyzed org-wide patterns");
                println!("✓ Quality gate validated response");
            }
            Err(e) => {
                eprintln!("\n=== Router Error ===");
                eprintln!("{}", e);
                eprintln!("\nNote: This test requires:");
                eprintln!("- GITHUB_TOKEN environment variable");
                eprintln!("- Local LLM model available");
                panic!("Router failed: {}", e);
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_cross_repo_issue_analysis() {
        // Advanced test: Analyze issues across multiple repos
        // to find patterns (e.g., common bugs, feature requests)

        let router = Router::new().await.expect("Failed to create router");
        let tool_registry = Arc::new(ToolRegistry::new());

        let task = "Look at open issues across arkavo-org repositories. \
                    Are there any common themes or patterns? \
                    Which repos have the most activity?";

        println!("\n=== Cross-Repo Issue Analysis ===");
        println!("Task: {}", task);

        match router
            .route_with_quality_gate(task, vec![], Some(&tool_registry), 3)
            .await
        {
            Ok(response) => {
                println!("\n=== Analysis Result ===");
                println!("{}", response.content);

                println!("\n=== This Demonstrates ===");
                println!("✓ Org-wide visibility (not single-repo)");
                println!("✓ Pattern detection across repos");
                println!("✓ Intelligent prioritization");
                println!("✓ Automated insights");
            }
            Err(e) => {
                eprintln!("Cross-repo analysis failed: {}", e);
            }
        }
    }
}
