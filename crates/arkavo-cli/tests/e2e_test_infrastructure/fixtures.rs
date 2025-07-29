use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use tempfile::TempDir;

pub struct TestEnvironment {
    pub temp_dir: TempDir,
    pub ui_port: u16,
    pub agent_ports: Vec<u16>,
    pub config_path: PathBuf,
}

impl TestEnvironment {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let temp_dir = TempDir::new()?;
        let ui_port = get_free_port()?;
        let config_path = temp_dir.path().join("AGENTS.md");

        // Create .arkavo directory for task database
        let arkavo_dir = temp_dir.path().join(".arkavo");
        std::fs::create_dir(&arkavo_dir)?;

        Ok(Self {
            temp_dir,
            ui_port,
            agent_ports: Vec::new(),
            config_path,
        })
    }

    pub fn with_agents(mut self, count: usize) -> Result<Self, Box<dyn std::error::Error>> {
        for _ in 0..count {
            self.agent_ports.push(get_free_port()?);
        }
        Ok(self)
    }

    pub fn create_agent_config(
        &self,
        agent_configs: &[AgentConfig],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut config_content = String::new();

        for (i, agent) in agent_configs.iter().enumerate() {
            let port = self
                .agent_ports
                .get(i)
                .copied()
                .unwrap_or_else(|| 8342 + i as u16);

            config_content.push_str(&format!(
                r#"## {}
purpose: {}
model: {}
listen: 0.0.0.0:{}
connect: http://127.0.0.1:{}

"#,
                agent.name, agent.purpose, agent.model, port, self.ui_port
            ));
        }

        fs::write(&self.config_path, config_content)?;
        Ok(())
    }
}

pub struct AgentConfig {
    pub name: String,
    pub purpose: String,
    pub model: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: "test-agent".to_string(),
            purpose: "Test agent for e2e testing".to_string(),
            model: "test-model".to_string(),
        }
    }
}

pub fn get_free_port() -> Result<u16, Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

pub fn create_test_message(content: &str) -> serde_json::Value {
    serde_json::json!({
        "message": content,
        "context": {
            "test": true,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }
    })
}
