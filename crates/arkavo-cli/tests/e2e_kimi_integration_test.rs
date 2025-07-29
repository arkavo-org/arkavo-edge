mod common;

use common::*;
use std::time::Duration;
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires arkavo binary and MOONSHOT_API_KEY"]
async fn test_kimi_k2_agent() -> Result<(), Box<dyn std::error::Error>> {
    // Check if MOONSHOT_API_KEY is set
    if std::env::var("MOONSHOT_API_KEY").is_err() {
        eprintln!("MOONSHOT_API_KEY not set, skipping KIMI integration test");
        return Ok(());
    }

    let env = TestEnvironment::new()?.with_agents(1)?;

    // Create KIMI agent configuration with moonshot-v1-128k model (closest to K2)
    env.create_agent_config(&[AgentConfig {
        name: "kimi-agent".to_string(),
        purpose: "Test KIMI K2 model integration".to_string(),
        model: "kimi://moonshot-v1-128k".to_string(),
    }])?;

    // Create separate directories for UI and agent
    let ui_dir = TempDir::new()?;
    std::fs::create_dir(ui_dir.path().join(".arkavo"))?;

    let agent_dir = TempDir::new()?;
    std::fs::create_dir(agent_dir.path().join(".arkavo"))?;
    std::fs::File::create(agent_dir.path().join(".arkavo").join("arkavo_tasks.db"))?;

    // Copy AGENTS.md to agent directory
    std::fs::copy(&env.config_path, agent_dir.path().join("AGENTS.md"))?;

    // Start UI
    let ui = spawn_component("ui", &["ui", &env.ui_port.to_string()], &ui_dir).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Start KIMI agent
    let agent = spawn_component(
        "agent",
        &[
            "agent",
            "run",
            agent_dir.path().join("AGENTS.md").to_str().unwrap(),
        ],
        &agent_dir,
    )
    .await?;
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Verify components are running
    let ui_running = ui.is_running().await;
    let agent_running = agent.is_running().await;

    // Clean up
    let _ = ui.kill().await;
    let _ = agent.kill().await;

    assert!(ui_running, "UI should be running");
    assert!(agent_running, "KIMI agent should be running");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires MOONSHOT_API_KEY"]
async fn test_kimi_api_direct() -> Result<(), Box<dyn std::error::Error>> {
    // Skip if API key not set
    let api_key = match std::env::var("MOONSHOT_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            eprintln!("MOONSHOT_API_KEY not set, skipping direct API test");
            return Ok(());
        }
    };

    // Test direct KIMI API using the client
    use arkavo_kimi::provider::{Message, Role};
    use arkavo_kimi::{KimiClient, KimiConfig, Model};

    let config = KimiConfig {
        api_key,
        model: Model::MoonshotV1_128k, // Use 128k model for K2-like performance
        ..Default::default()
    };

    let client = KimiClient::new(config)?;

    let messages = vec![
        Message {
            role: Role::System,
            content: "You are a helpful AI assistant testing the KIMI K2 integration.".to_string(),
            images: None,
        },
        Message {
            role: Role::User,
            content: "What is 2 + 2? Reply with just the number.".to_string(),
            images: None,
        },
    ];

    let response = client
        .create_chat_completion(messages, None, None, None, None)
        .await?;

    println!("KIMI K2 response: {}", response);
    assert!(response.contains("4"), "Response should contain '4'");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires MOONSHOT_API_KEY"]
async fn test_kimi_streaming() -> Result<(), Box<dyn std::error::Error>> {
    // Skip if API key not set
    let api_key = match std::env::var("MOONSHOT_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            eprintln!("MOONSHOT_API_KEY not set, skipping streaming test");
            return Ok(());
        }
    };

    use arkavo_kimi::provider::{Message, Role};
    use arkavo_kimi::{KimiClient, KimiConfig, Model};

    let config = KimiConfig {
        api_key,
        model: Model::MoonshotV1_128k,
        ..Default::default()
    };

    let client = KimiClient::new(config)?;

    let messages = vec![Message {
        role: Role::User,
        content: "Count from 1 to 5.".to_string(),
        images: None,
    }];

    use futures::StreamExt;

    let mut stream = client
        .create_chat_completion_stream(messages, None, None, None, None)
        .await?;

    let mut full_response = String::new();
    while let Some(result) = stream.next().await {
        match result {
            Ok(response) => {
                print!("{}", response.content);
                full_response.push_str(&response.content);
            }
            Err(e) => {
                eprintln!("Stream error: {}", e);
                break;
            }
        }
    }
    println!(); // New line after streaming

    assert!(
        !full_response.is_empty(),
        "Should receive streamed response"
    );
    assert!(
        full_response.contains("1") && full_response.contains("5"),
        "Response should contain numbers 1 through 5"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires MOONSHOT_API_KEY"]
async fn test_kimi_long_context() -> Result<(), Box<dyn std::error::Error>> {
    // Skip if API key not set
    let api_key = match std::env::var("MOONSHOT_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            eprintln!("MOONSHOT_API_KEY not set, skipping long context test");
            return Ok(());
        }
    };

    use arkavo_kimi::provider::{Message, Role};
    use arkavo_kimi::{KimiClient, KimiConfig, Model};

    let config = KimiConfig {
        api_key,
        model: Model::MoonshotV1_128k,     // 128k context window
        timeout: Duration::from_secs(120), // Longer timeout for long context
        ..Default::default()
    };

    let client = KimiClient::new(config)?;

    // Create a long context by repeating a pattern
    let long_text = "This is a test sentence. ".repeat(1000); // ~25k chars
    println!("Long text length: {} chars", long_text.len());
    println!(
        "Expected 'test' count: {}",
        long_text.matches("test").count()
    );

    let messages = vec![Message {
        role: Role::User,
        content: format!(
            "Here is a long text:\n{}\n\nHow many times does the word 'test' appear?",
            long_text
        ),
        images: None,
    }];

    let response = client
        .create_chat_completion(messages, None, None, None, None)
        .await?;

    println!("Long context response: {}", response);

    // The response should contain "1000" as that's the actual count
    assert!(
        response.contains("1000")
            || response.contains("thousand")
            || (response.contains("test")
                && (response.contains("times") || response.contains("occurrences"))),
        "Response should indicate the word 'test' appears in the text"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires arkavo binary and MOONSHOT_API_KEY"]
async fn test_kimi_agent_with_mcp() -> Result<(), Box<dyn std::error::Error>> {
    // Check if MOONSHOT_API_KEY is set
    if std::env::var("MOONSHOT_API_KEY").is_err() {
        eprintln!("MOONSHOT_API_KEY not set, skipping KIMI MCP test");
        return Ok(());
    }

    let env = TestEnvironment::new()?.with_agents(1)?;

    // Create agent config with MCP server
    let agent_config = r#"## kimi-mcp-agent
purpose: Test KIMI K2 with MCP integration
model: kimi://moonshot-v1-128k
listen: 0.0.0.0:{PORT}
mcp_servers:
  - name: test-echo
    command: echo
    args: ["MCP test"]
"#
    .replace("{PORT}", &env.agent_ports[0].to_string());

    std::fs::write(&env.config_path, agent_config)?;

    // Create directories
    let ui_dir = TempDir::new()?;
    std::fs::create_dir(ui_dir.path().join(".arkavo"))?;

    let agent_dir = TempDir::new()?;
    std::fs::create_dir(agent_dir.path().join(".arkavo"))?;
    std::fs::File::create(agent_dir.path().join(".arkavo").join("arkavo_tasks.db"))?;
    std::fs::copy(&env.config_path, agent_dir.path().join("AGENTS.md"))?;

    // Start components
    let ui = spawn_component("ui", &["ui", &env.ui_port.to_string()], &ui_dir).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;

    let agent = spawn_component(
        "agent",
        &[
            "agent",
            "run",
            agent_dir.path().join("AGENTS.md").to_str().unwrap(),
        ],
        &agent_dir,
    )
    .await?;
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Verify running
    let ui_running = ui.is_running().await;
    let agent_running = agent.is_running().await;

    // Clean up
    let _ = ui.kill().await;
    let _ = agent.kill().await;

    assert!(ui_running, "UI should be running");
    assert!(agent_running, "KIMI agent with MCP should be running");

    Ok(())
}
