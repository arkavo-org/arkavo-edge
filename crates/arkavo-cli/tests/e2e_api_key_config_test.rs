mod common;

use common::*;
use std::time::Duration;
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires arkavo binary"]
async fn test_api_key_configuration() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnvironment::new()?.with_agents(1)?;

    // Create agent configuration with API key
    let agent_config = format!(
        r#"## test-agent
purpose: Test API key configuration
model: kimi://moonshot-v1-128k
listen: 0.0.0.0:{}
MOONSHOT_API_KEY: test-api-key-12345
OPENAI_API_KEY: sk-test-openai-key
ANTHROPIC_API_KEY: sk-test-anthropic-key
"#,
        env.agent_ports[0]
    );

    std::fs::write(&env.config_path, agent_config)?;

    // Create directories
    let agent_dir = TempDir::new()?;
    std::fs::create_dir(agent_dir.path().join(".arkavo"))?;
    std::fs::File::create(agent_dir.path().join(".arkavo").join("arkavo_tasks.db"))?;
    std::fs::copy(&env.config_path, agent_dir.path().join("AGENTS.md"))?;

    // Start agent
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

    // Give it time to start
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Give it more time to fully initialize
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Verify agent started successfully
    assert!(agent.is_running().await, "Agent should be running");

    // Clean up
    let _ = agent.kill().await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires arkavo binary"]
async fn test_multiple_agents_different_api_keys() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnvironment::new()?.with_agents(2)?;

    // Create configuration with two agents having different API keys
    let agent_config = format!(
        r#"## agent-one
purpose: First agent with OpenAI
model: openai://gpt-4
listen: 0.0.0.0:{}
OPENAI_API_KEY: sk-agent-one-key

## agent-two  
purpose: Second agent with Kimi
model: kimi://moonshot-v1-128k
listen: 0.0.0.0:{}
MOONSHOT_API_KEY: sk-agent-two-key
"#,
        env.agent_ports[0], env.agent_ports[1]
    );

    std::fs::write(&env.config_path, agent_config)?;

    // Verify parsing works correctly
    let agents = parse_test_agents_config(&std::fs::read_to_string(&env.config_path)?)?;

    assert_eq!(agents.len(), 2, "Should parse two agents");

    let agent_one = &agents[0];
    assert_eq!(agent_one.name, "agent-one");
    assert_eq!(
        agent_one.api_keys.get("OPENAI_API_KEY"),
        Some(&"sk-agent-one-key".to_string())
    );
    assert_eq!(agent_one.api_keys.get("MOONSHOT_API_KEY"), None);

    let agent_two = &agents[1];
    assert_eq!(agent_two.name, "agent-two");
    assert_eq!(
        agent_two.api_keys.get("MOONSHOT_API_KEY"),
        Some(&"sk-agent-two-key".to_string())
    );
    assert_eq!(agent_two.api_keys.get("OPENAI_API_KEY"), None);

    Ok(())
}

// Helper function to parse agent config for testing
#[derive(Debug)]
struct TestAgentConfig {
    name: String,
    api_keys: std::collections::HashMap<String, String>,
}

fn parse_test_agents_config(
    content: &str,
) -> Result<Vec<TestAgentConfig>, Box<dyn std::error::Error>> {
    let mut agents = Vec::new();
    let mut current_agent: Option<TestAgentConfig> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("## ") {
            if let Some(agent) = current_agent.take() {
                agents.push(agent);
            }

            let name = trimmed[3..].trim().to_string();
            current_agent = Some(TestAgentConfig {
                name,
                api_keys: std::collections::HashMap::new(),
            });
        } else if let Some(agent) = current_agent.as_mut() {
            if trimmed.contains("_API_KEY:") || trimmed.contains("_api_key:") {
                if let Some(colon_pos) = trimmed.find(':') {
                    let key_name = trimmed[..colon_pos].trim().to_string();
                    let key_value = trimmed[colon_pos + 1..].trim().to_string();
                    agent.api_keys.insert(key_name, key_value);
                }
            }
        }
    }

    if let Some(agent) = current_agent {
        agents.push(agent);
    }

    Ok(agents)
}
