use arkavo_test::{TestHarness, TestError};
use arkavo_test::gherkin::parser::Parser;
use arkavo_test::execution::runner::TestRunner;
use std::path::Path;

#[tokio::main]
async fn main() -> Result<(), TestError> {
    println!("=== Arkavo Test Harness Demo ===\n");

    // Initialize test harness
    let _harness = TestHarness::new()?;
    
    // Parse the banking app feature file
    let feature_path = Path::new("crates/arkavo-test/examples/banking_app.feature");
    let feature = Parser::parse_feature_file(feature_path)?;
    
    println!("Feature: {}", feature.name);
    if let Some(desc) = &feature.description {
        println!("Description: {}", desc);
    }
    println!("Scenarios: {}", feature.scenarios.len());
    println!();

    // Run the first scenario
    if let Some(scenario) = feature.scenarios.first() {
        println!("Running scenario: {}", scenario.name);
        
        let runner = TestRunner::new();
        let result = runner.run_scenario(scenario.clone()).await?;
        
        use arkavo_test::reporting::business_report::TestStatus;
        
        match result.status {
            TestStatus::Passed => {
                println!("✅ Scenario passed in {:?}", result.duration);
            }
            TestStatus::Failed => {
                println!("❌ Scenario failed");
                for (i, step) in result.steps.iter().enumerate() {
                    println!("  Step {}: {} - {:?}", i + 1, step.text, step.status);
                }
            }
            TestStatus::Skipped => {
                println!("⏭️  Scenario skipped");
            }
            TestStatus::Pending => {
                println!("⏳ Scenario pending");
            }
        }
    }
    
    // Demo MCP server integration
    println!("\n=== MCP Server Demo ===\n");
    
    let harness = TestHarness::new()?;
    let mcp_server = harness.mcp_server();
    
    // Query state
    use arkavo_test::mcp::server::ToolRequest;
    use serde_json::json;
    
    let query = ToolRequest {
        tool_name: "query_state".to_string(),
        params: json!({
            "entity": "test_status"
        }),
    };
    
    match mcp_server.call_tool(query).await {
        Ok(response) => {
            println!("MCP Query Response: {:?}", response.result);
        }
        Err(e) => {
            println!("MCP Query Error: {}", e);
        }
    }
    
    println!("\nDemo completed!");
    Ok(())
}