mod e2e_test_infrastructure;

use std::collections::HashMap;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires manual testing with arkavo binary"]
async fn test_api_key_parsing() -> Result<(), Box<dyn std::error::Error>> {
    // Test configuration with API keys
    let agent_config = r#"## test-agent
purpose: Test API key configuration
model: kimi://moonshot-v1-128k
listen: 0.0.0.0:8342
MOONSHOT_API_KEY: test-api-key-12345
OPENAI_API_KEY: sk-test-openai-key
ANTHROPIC_API_KEY: sk-test-anthropic-key
"#;

    // Test parsing works correctly
    let agents = parse_test_agents_config(agent_config)?;

    assert_eq!(agents.len(), 1, "Should parse one agent");

    let agent = &agents[0];
    assert_eq!(agent.name, "test-agent");
    assert_eq!(
        agent.api_keys.get("MOONSHOT_API_KEY"),
        Some(&"test-api-key-12345".to_string())
    );
    assert_eq!(
        agent.api_keys.get("OPENAI_API_KEY"),
        Some(&"sk-test-openai-key".to_string())
    );
    assert_eq!(
        agent.api_keys.get("ANTHROPIC_API_KEY"),
        Some(&"sk-test-anthropic-key".to_string())
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_multiple_agents_different_api_keys() -> Result<(), Box<dyn std::error::Error>> {
    // Create configuration with two agents having different API keys
    let agent_config = r#"## agent-one
purpose: First agent with OpenAI
model: openai://gpt-4
listen: 0.0.0.0:8343
OPENAI_API_KEY: sk-agent-one-key

## agent-two  
purpose: Second agent with Kimi
model: kimi://moonshot-v1-128k
listen: 0.0.0.0:8344
MOONSHOT_API_KEY: sk-agent-two-key
"#;

    // Verify parsing works correctly
    let agents = parse_test_agents_config(agent_config)?;

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
    api_keys: HashMap<String, String>,
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
                api_keys: HashMap::new(),
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
