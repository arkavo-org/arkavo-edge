use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

use super::config::{TaskConfig, TaskResult};
use super::strategy::TaskStrategy;
use super::ui::TaskUI;
use crate::error::{Error, Result};

use arkavo_protocol::{
    agent_registry::{AgentInfo, AgentRegistry},
    http::HttpTransport,
    transport::{A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, TlsConfig, TransportConfig},
    types::{
        Message, MessagePart, MessageSendRequest, MessageSendResponse, TaskGetRequest,
        TaskGetResponse, TaskStatus,
    },
};

/// Mesh task execution strategy
///
/// Delegates tasks to agents discovered on the mesh network
/// via mDNS and A2A protocol.
pub struct MeshTaskStrategy {
    /// Agent registry for load balancing
    registry: Arc<AgentRegistry>,
}

impl Default for MeshTaskStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl MeshTaskStrategy {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(AgentRegistry::new()),
        }
    }

    /// Discover agents on the mesh network using mDNS
    #[cfg(feature = "mdns")]
    pub fn discover_agents() -> Result<Vec<AgentInfo>> {
        use mdns_sd::{ServiceDaemon, ServiceEvent};
        use std::collections::HashMap;

        tracing::info!("Discovering mesh agents via mDNS...");

        let mdns = ServiceDaemon::new()
            .map_err(|e| Error::Other(anyhow::anyhow!("Failed to create mDNS daemon: {e}")))?;
        let receiver = mdns
            .browse("_a2a._tcp.local.")
            .map_err(|e| Error::Other(anyhow::anyhow!("Failed to browse mDNS: {e}")))?;

        let mut agents = Vec::new();
        let timeout = Duration::from_secs(3);
        let start = std::time::Instant::now();

        while start.elapsed() < timeout {
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(event) => {
                    if let ServiceEvent::ServiceResolved(info) = event {
                        let agent_id = info
                            .get_property_val_str("agent_id")
                            .unwrap_or("unknown")
                            .to_string();
                        let name = info.get_fullname().to_string();
                        let purpose = info
                            .get_property_val_str("purpose")
                            .unwrap_or("")
                            .to_string();

                        let capabilities_str =
                            info.get_property_val_str("capabilities").unwrap_or_default();
                        let mut capabilities: Vec<String> = if capabilities_str.is_empty() {
                            vec![]
                        } else {
                            capabilities_str.split(',').map(|s| s.to_string()).collect()
                        };

                        let mcp_tools_str =
                            info.get_property_val_str("mcp_tools").unwrap_or_default();
                        if !mcp_tools_str.is_empty() {
                            let mcp_tools: Vec<String> =
                                mcp_tools_str.split(',').map(|s| s.to_string()).collect();
                            capabilities.extend(mcp_tools);
                        }

                        let address = info
                            .get_addresses()
                            .iter()
                            .next()
                            .map(|addr| format!("http://{}:{}", addr, info.get_port()));

                        let mut metadata = HashMap::new();
                        if let Some(model) = info.get_property_val_str("model") {
                            metadata.insert("model".to_string(), model.to_string());
                        }

                        agents.push(AgentInfo {
                            agent_id,
                            name: name.clone(),
                            purpose,
                            capabilities,
                            device_caps: None,
                            metadata,
                            last_seen: chrono::Utc::now(),
                            load: 0.0,
                            is_available: true,
                            address,
                        });

                        tracing::debug!("Discovered agent: {}", name);
                    }
                }
                Err(_) => {
                    // Timeout on recv - continue until overall timeout
                }
            }
        }

        tracing::info!("Discovered {} agents via mDNS", agents.len());
        Ok(agents)
    }

    /// Fallback when mDNS feature is not compiled
    #[cfg(not(feature = "mdns"))]
    pub fn discover_agents() -> Result<Vec<AgentInfo>> {
        tracing::warn!("mDNS feature not compiled in - cannot discover agents");
        Ok(Vec::new())
    }

    /// Select best agent for the task
    async fn select_agent<'a>(
        &self,
        agents: &'a [AgentInfo],
        config: &TaskConfig,
    ) -> Result<&'a AgentInfo> {
        // If target agent specified, use it
        if let Some(target_id) = &config.target_agent_id {
            return agents
                .iter()
                .find(|a| &a.agent_id == target_id)
                .ok_or_else(|| {
                    Error::Other(anyhow::anyhow!("Target agent {} not found", target_id))
                });
        }

        // Register all discovered agents
        for agent in agents {
            let _ = self
                .registry
                .register_agent(
                    agent.agent_id.clone(),
                    agent.name.clone(),
                    agent.purpose.clone(),
                    agent.capabilities.clone(),
                    agent.device_caps.clone(),
                    agent.metadata.clone(),
                    agent.address.clone(),
                )
                .await;
        }

        // Find best agent for code generation
        let best_agent_id = self
            .registry
            .find_best_agent("code_generation")
            .await
            .or_else(|| agents.first().map(|a| a.agent_id.clone()))
            .ok_or_else(|| Error::Other(anyhow::anyhow!("No suitable agent found")))?;

        agents
            .iter()
            .find(|a| a.agent_id == best_agent_id)
            .ok_or_else(|| Error::Other(anyhow::anyhow!("Selected agent not found")))
    }

    /// Submit task to agent and monitor until completion
    async fn submit_and_monitor(
        &self,
        task: &str,
        agent: &AgentInfo,
        config: &TaskConfig,
        ui: &dyn TaskUI,
    ) -> Result<TaskResult> {
        let address = agent
            .address
            .as_ref()
            .ok_or_else(|| Error::Other(anyhow::anyhow!("Agent has no address")))?;

        ui.section("Connecting to Agent");
        ui.status(&format!("Address: {address}"));

        // Create transport
        let transport_config = TransportConfig {
            timeout_ms: 60000,
            max_retries: 2,
            tls_config: TlsConfig {
                require_tls: false,
                ..Default::default()
            },
            ..Default::default()
        };

        let transport = Arc::new(
            HttpTransport::new(transport_config)
                .map_err(|e| Error::Other(anyhow::anyhow!("Failed to create transport: {e}")))?,
        );

        // Create endpoint
        let endpoint = A2aEndpoint {
            url: address.clone(),
            agent_id: agent.agent_id.clone(),
            public_key: None,
        };

        // Connect
        transport
            .connect(&endpoint)
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("Failed to connect: {e}")))?;

        ui.success("Connected");

        // Build task message
        ui.section("Submitting Task");
        ui.status(&format!("Task: {task}"));

        let task_prompt = format!(
            "Task: {task}\n\nPlease analyze the repository, plan the changes, and execute the task. Use MCP tools to read files, make changes, and verify results."
        );

        let message = Message {
            parts: vec![MessagePart::Text {
                content: task_prompt,
            }],
            metadata: Some(serde_json::json!({
                "task_type": "code_task",
                "source": "arkavo_mesh_orchestrator",
                "auto_execute": config.auto_approve,
            })),
        };

        let send_request = MessageSendRequest {
            message,
            task_id: None,
        };

        let rpc_request = A2aRequest::new("message/send", serde_json::json!([send_request]));

        let response = transport
            .send_request(rpc_request)
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("Failed to send task: {e}")))?;

        let task_id = match response {
            A2aResponse::Success { result, .. } => {
                let send_response: MessageSendResponse = serde_json::from_value(result)
                    .map_err(|e| Error::Other(anyhow::anyhow!("Failed to parse response: {e}")))?;
                ui.success("Task submitted!");
                ui.status(&format!("Task ID: {}", send_response.task_id));
                ui.status(&format!("Status: {:?}", send_response.status));
                send_response.task_id
            }
            A2aResponse::Error { error, .. } => {
                let _ = transport.close().await;
                return Err(Error::Other(anyhow::anyhow!(
                    "RPC error: {} - {}",
                    error.code,
                    error.message
                )));
            }
        };

        // Poll for completion
        ui.section("Monitoring Task Progress");

        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(300); // 5 minutes

        loop {
            if start.elapsed() > timeout {
                let _ = transport.close().await;
                return Err(Error::Other(anyhow::anyhow!(
                    "Task execution timed out after 5 minutes"
                )));
            }

            let get_request = TaskGetRequest {
                task_id: task_id.clone(),
            };

            let rpc_request = A2aRequest::new("tasks/get", serde_json::json!([get_request]));

            let response = transport
                .send_request(rpc_request)
                .await
                .map_err(|e| Error::Other(anyhow::anyhow!("Failed to get task status: {e}")))?;

            match response {
                A2aResponse::Success { result, .. } => {
                    let task_response: TaskGetResponse = serde_json::from_value(result)
                        .map_err(|e| Error::Other(anyhow::anyhow!("Failed to parse: {e}")))?;

                    match task_response.status {
                        TaskStatus::Completed => {
                            let _ = transport.close().await;
                            ui.success("Task completed successfully!");
                            return Ok(TaskResult::success("Completed via mesh agent"));
                        }
                        TaskStatus::Failed => {
                            let _ = transport.close().await;
                            ui.error("Task failed");
                            return Ok(TaskResult::failure("Task failed on mesh agent"));
                        }
                        TaskStatus::Canceled => {
                            let _ = transport.close().await;
                            return Ok(TaskResult::failure("Task was cancelled"));
                        }
                        status => {
                            // Show progress if available
                            if let Some(progress) = &task_response.progress {
                                if let Some(msg) = &progress.message {
                                    ui.progress(msg, progress.percentage);
                                }
                            }
                            ui.status(&format!("Status: {:?}", status));
                        }
                    }
                }
                A2aResponse::Error { error, .. } => {
                    let _ = transport.close().await;
                    return Err(Error::Other(anyhow::anyhow!(
                        "RPC error: {} - {}",
                        error.code,
                        error.message
                    )));
                }
            }

            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
}

#[async_trait]
impl TaskStrategy for MeshTaskStrategy {
    async fn is_available(&self) -> bool {
        match Self::discover_agents() {
            Ok(agents) => !agents.is_empty(),
            Err(_) => false,
        }
    }

    async fn execute(
        &self,
        task: &str,
        config: &TaskConfig,
        ui: &dyn TaskUI,
    ) -> Result<TaskResult> {
        // Discover agents
        let agents = Self::discover_agents()?;

        if agents.is_empty() {
            return Err(Error::Other(anyhow::anyhow!("No mesh agents discovered")));
        }

        // Display discovered agents
        ui.section("Discovered Mesh Agents");
        for agent in &agents {
            ui.status(&format!("  - {} ({})", agent.name, agent.agent_id));
            if !agent.purpose.is_empty() {
                ui.status(&format!("    Purpose: {}", agent.purpose));
            }
            if !agent.capabilities.is_empty() {
                ui.status(&format!("    Capabilities: {}", agent.capabilities.join(", ")));
            }
        }

        // Select agent
        let agent = self.select_agent(&agents, config).await?;

        ui.section("Selected Agent");
        ui.status(&format!("Agent: {} ({})", agent.name, agent.agent_id));
        ui.status(&format!("Load: {:.0}%", agent.load * 100.0));

        // Submit and monitor
        self.submit_and_monitor(task, agent, config, ui).await
    }

    fn name(&self) -> &str {
        "mesh"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_executor::MockUI;

    #[tokio::test]
    async fn test_mesh_strategy_no_agents() {
        let strategy = MeshTaskStrategy::new();
        let ui = MockUI::new();
        let config = TaskConfig::default();

        // Without mDNS or real agents, this should fail gracefully
        let result = strategy.execute("test task", &config, &ui).await;

        // Either fails with no agents or succeeds if agents exist
        // We can't assert specific outcome since it depends on environment
        match result {
            Ok(_) => {} // Agents were available
            Err(e) => {
                // Should be "no agents" error
                assert!(e.to_string().contains("agent") || e.to_string().contains("mDNS"));
            }
        }
    }

    #[test]
    fn test_discover_agents_without_mdns() {
        // When mdns feature is disabled, should return empty
        #[cfg(not(feature = "mdns"))]
        {
            let agents = MeshTaskStrategy::discover_agents().unwrap();
            assert!(agents.is_empty());
        }
    }
}
