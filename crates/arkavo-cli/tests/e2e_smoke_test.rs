mod e2e_test_infrastructure;

use e2e_test_infrastructure::*;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires arkavo binary to be built"]
async fn test_agent_ui_echo() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnvironment::new()?.with_agents(1)?;

    env.create_agent_config(&[AgentConfig {
        name: "echo-agent".to_string(),
        purpose: "Echo test agent".to_string(),
        model: "test-model".to_string(),
    }])?;

    // Create separate temp directories for UI and agent to avoid database conflicts
    let ui_dir = tempfile::TempDir::new()?;
    std::fs::create_dir(ui_dir.path().join(".arkavo"))?;

    let agent_dir = tempfile::TempDir::new()?;
    std::fs::create_dir(agent_dir.path().join(".arkavo"))?;

    // Create empty database file to avoid SQLite issues
    let db_path = agent_dir.path().join(".arkavo").join("arkavo_tasks.db");
    std::fs::File::create(&db_path)?;

    // Copy AGENTS.md to agent directory
    std::fs::copy(&env.config_path, agent_dir.path().join("AGENTS.md"))?;

    let ui = spawn_component("ui", &["ui", &env.ui_port.to_string()], &ui_dir).await?;
    // UI doesn't have a health endpoint, just wait a bit for it to start
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
    // Agent doesn't have an HTTP health endpoint, wait for it to start
    tokio::time::sleep(Duration::from_secs(2)).await;

    // For now, just verify the components started successfully
    let ui_running = ui.is_running().await;
    let agent_running = agent.is_running().await;

    // Always clean up
    let _ = ui.kill().await;
    let _ = agent.kill().await;

    // Now assert
    assert!(ui_running, "UI should be running");
    assert!(agent_running, "Agent should be running");

    // Test basic API connectivity
    // Send a simple request to the UI endpoint
    let ui_url = "http://localhost:3000/health";
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    // Try to connect with retries
    let mut connected = false;
    for _ in 0..5 {
        if let Ok(response) = client.get(ui_url).send().await
            && response.status().is_success() {
                connected = true;
                break;
            }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }

    assert!(connected, "Should be able to connect to UI API");

    Ok(())
}
