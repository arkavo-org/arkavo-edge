use arkavo_test::mcp::server::ToolRequest;
use arkavo_test::{TestError, TestHarness};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), TestError> {
    println!("Starting Arkavo Test MCP Server demo...\n");

    // Initialize the test harness
    let harness = TestHarness::new()?;

    // Get access to the MCP server
    let mcp_server = harness.mcp_server();

    // Demonstrate MCP tools
    println!("Available MCP tools:");
    println!("  - query_state: Query application state");
    println!("  - mutate_state: Modify application state");
    println!("  - snapshot: Create/restore state snapshots");
    println!("  - run_test: Execute test scenarios");
    println!();

    // Test 1: Query initial state
    println!("Test 1: Querying initial application state");
    let query_request = ToolRequest {
        tool_name: "query_state".to_string(),
        params: json!({
            "entity": "user",
            "filter": {
                "field": "balance"
            }
        }),
    };

    match mcp_server.call_tool(query_request).await {
        Ok(response) => println!("Response: {}\n", serde_json::to_string_pretty(&response)?),
        Err(e) => println!("Error: {}\n", e),
    }

    // Test 2: Run a Gherkin test
    println!("Test 2: Running Gherkin scenario");
    let test_request = ToolRequest {
        tool_name: "run_test".to_string(),
        params: json!({
            "test_name": "banking_app::successful_withdrawal",
            "timeout": 30
        }),
    };

    match mcp_server.call_tool(test_request).await {
        Ok(response) => println!("Response: {}\n", serde_json::to_string_pretty(&response)?),
        Err(e) => println!("Error: {}\n", e),
    }

    // Test 3: Create a snapshot
    println!("Test 3: Creating application snapshot");
    let snapshot_request = ToolRequest {
        tool_name: "snapshot".to_string(),
        params: json!({
            "action": "create",
            "name": "test_checkpoint_1"
        }),
    };

    match mcp_server.call_tool(snapshot_request).await {
        Ok(response) => println!("Response: {}\n", serde_json::to_string_pretty(&response)?),
        Err(e) => println!("Error: {}\n", e),
    }

    // Test 4: Mutate state
    println!("Test 4: Mutating application state");
    let mutate_request = ToolRequest {
        tool_name: "mutate_state".to_string(),
        params: json!({
            "entity": "user",
            "action": "update_balance",
            "data": {
                "balance": 500
            }
        }),
    };

    match mcp_server.call_tool(mutate_request).await {
        Ok(response) => println!("Response: {}\n", serde_json::to_string_pretty(&response)?),
        Err(e) => println!("Error: {}\n", e),
    }

    // Test 5: List snapshots
    println!("Test 5: Listing available snapshots");
    let list_request = ToolRequest {
        tool_name: "snapshot".to_string(),
        params: json!({
            "action": "list"
        }),
    };

    match mcp_server.call_tool(list_request).await {
        Ok(response) => println!("Response: {}\n", serde_json::to_string_pretty(&response)?),
        Err(e) => println!("Error: {}\n", e),
    }

    println!("MCP Server demo completed!");
    Ok(())
}
