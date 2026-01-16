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

        let mdns = ServiceDaemon::new().map_err(|e| Error::Discovery {
            operation: "create mDNS daemon".to_string(),
            details: e.to_string(),
        })?;
        let receiver = mdns
            .browse("_a2a._tcp.local.")
            .map_err(|e| Error::Discovery {
                operation: "browse A2A services".to_string(),
                details: e.to_string(),
            })?;

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

                        let capabilities_str = info
                            .get_property_val_str("capabilities")
                            .unwrap_or_default();
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

    /// Infer required capabilities from task content
    fn infer_capabilities(task: &str) -> Vec<&'static str> {
        let task_lower = task.to_lowercase();
        let mut capabilities = Vec::new();

        // Security-related keywords
        if task_lower.contains("security")
            || task_lower.contains("vulnerab")
            || task_lower.contains("auth")
            || task_lower.contains("injection")
            || task_lower.contains("xss")
            || task_lower.contains("csrf")
        {
            capabilities.push("security_analysis");
            capabilities.push("vulnerability_detection");
        }

        // Testing-related keywords
        if task_lower.contains("test")
            || task_lower.contains("coverage")
            || task_lower.contains("unit test")
            || task_lower.contains("integration test")
        {
            capabilities.push("test_generation");
            capabilities.push("coverage_analysis");
        }

        // Performance-related keywords
        if task_lower.contains("performance")
            || task_lower.contains("optimize")
            || task_lower.contains("profil")
            || task_lower.contains("bottleneck")
            || task_lower.contains("slow")
        {
            capabilities.push("performance_analysis");
            capabilities.push("optimization");
        }

        // Database-related keywords
        if task_lower.contains("database")
            || task_lower.contains("sql")
            || task_lower.contains("query")
            || task_lower.contains("schema")
            || task_lower.contains("index")
        {
            capabilities.push("database_optimization");
            capabilities.push("schema_design");
        }

        // Documentation-related keywords
        if task_lower.contains("document")
            || task_lower.contains("readme")
            || task_lower.contains("api doc")
            || task_lower.contains("comment")
        {
            capabilities.push("documentation_generation");
            capabilities.push("api_documentation");
        }

        // Architecture-related keywords
        if task_lower.contains("architect")
            || task_lower.contains("design")
            || task_lower.contains("scalab")
            || task_lower.contains("microservice")
        {
            capabilities.push("system_design");
            capabilities.push("scalability_patterns");
        }

        // Code review keywords
        if task_lower.contains("review")
            || task_lower.contains("refactor")
            || task_lower.contains("pattern")
            || task_lower.contains("quality")
        {
            capabilities.push("code_review");
            capabilities.push("pattern_analysis");
        }

        // Frontend-related keywords
        if task_lower.contains("frontend")
            || task_lower.contains("ui")
            || task_lower.contains("ux")
            || task_lower.contains("accessib")
            || task_lower.contains("responsive")
        {
            capabilities.push("ui_ux_analysis");
            capabilities.push("accessibility");
        }

        // DevOps-related keywords
        if task_lower.contains("devops")
            || task_lower.contains("ci/cd")
            || task_lower.contains("deploy")
            || task_lower.contains("pipeline")
            || task_lower.contains("docker")
            || task_lower.contains("kubernetes")
        {
            capabilities.push("ci_cd");
            capabilities.push("deployment_strategies");
        }

        // Data science keywords
        if task_lower.contains("machine learning")
            || task_lower.contains("ml")
            || task_lower.contains("data analy")
            || task_lower.contains("model")
            || task_lower.contains("feature")
        {
            capabilities.push("data_analysis");
            capabilities.push("ml_modeling");
        }

        // Default fallback
        if capabilities.is_empty() {
            capabilities.push("code_generation");
            capabilities.push("code_review");
        }

        capabilities
    }

    /// Select best agent for the task based on task content analysis
    async fn select_agent<'a>(
        &self,
        task: &str,
        agents: &'a [AgentInfo],
        config: &TaskConfig,
    ) -> Result<&'a AgentInfo> {
        // If target agent specified, use it
        if let Some(target_id) = &config.target_agent_id {
            return agents
                .iter()
                .find(|a| &a.agent_id == target_id)
                .ok_or_else(|| Error::AgentCommunication {
                    operation: "select target agent".to_string(),
                    agent_id: target_id.clone(),
                    details: "target agent not found in discovered agents".to_string(),
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

        // Infer required capabilities from task content
        let required_capabilities = Self::infer_capabilities(task);
        tracing::debug!(
            "Inferred capabilities for task: {:?}",
            required_capabilities
        );

        // Try each inferred capability in order
        for capability in &required_capabilities {
            if let Some(agent_id) = self.registry.find_best_agent(capability).await {
                tracing::info!(
                    "Selected agent '{}' for capability '{}'",
                    agent_id,
                    capability
                );
                if let Some(agent) = agents.iter().find(|a| a.agent_id == agent_id) {
                    return Ok(agent);
                }
            }
        }

        // Fallback: find agent by matching purpose keywords
        let task_lower = task.to_lowercase();
        for agent in agents {
            let purpose_lower = agent.purpose.to_lowercase();
            // Check if any significant word from the task appears in the agent's purpose
            for word in task_lower.split_whitespace() {
                if word.len() > 4 && purpose_lower.contains(word) {
                    tracing::info!(
                        "Selected agent '{}' by purpose keyword match: '{}'",
                        agent.agent_id,
                        word
                    );
                    return Ok(agent);
                }
            }
        }

        // Last resort: first agent
        agents.first().ok_or_else(|| Error::TaskExecution {
            operation: "select agent for task".to_string(),
            details: "no suitable agent found with required capabilities".to_string(),
        })
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
            .ok_or_else(|| Error::AgentCommunication {
                operation: "connect to agent".to_string(),
                agent_id: agent.agent_id.clone(),
                details: "agent has no address configured".to_string(),
            })?;

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

        let transport = Arc::new(HttpTransport::new(transport_config).map_err(|e| {
            Error::AgentCommunication {
                operation: "create HTTP transport".to_string(),
                agent_id: agent.agent_id.clone(),
                details: e.to_string(),
            }
        })?);

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
            .map_err(|e| Error::AgentCommunication {
                operation: "establish connection".to_string(),
                agent_id: agent.agent_id.clone(),
                details: e.to_string(),
            })?;

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

        let response =
            transport
                .send_request(rpc_request)
                .await
                .map_err(|e| Error::TaskExecution {
                    operation: format!("send task to agent '{}'", agent.agent_id),
                    details: e.to_string(),
                })?;

        let task_id = match response {
            A2aResponse::Success { result, .. } => {
                let send_response: MessageSendResponse =
                    serde_json::from_value(result).map_err(|e| Error::TaskExecution {
                        operation: "parse agent response".to_string(),
                        details: e.to_string(),
                    })?;
                ui.success("Task submitted!");
                ui.status(&format!("Task ID: {}", send_response.task_id));
                ui.status(&format!("Status: {:?}", send_response.status));
                send_response.task_id
            }
            A2aResponse::Error { error, .. } => {
                let _ = transport.close().await;
                return Err(Error::TaskExecution {
                    operation: "submit task via mesh".to_string(),
                    details: format!("RPC error {}: {}", error.code, error.message),
                });
            }
        };

        // Poll for completion
        ui.section("Monitoring Task Progress");

        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(300); // 5 minutes

        loop {
            if start.elapsed() > timeout {
                let _ = transport.close().await;
                return Err(Error::TaskExecution {
                    operation: "execute task via mesh".to_string(),
                    details: "task execution timed out after 5 minutes".to_string(),
                });
            }

            let get_request = TaskGetRequest {
                task_id: task_id.clone(),
            };

            let rpc_request = A2aRequest::new("tasks/get", serde_json::json!([get_request]));

            let response =
                transport
                    .send_request(rpc_request)
                    .await
                    .map_err(|e| Error::TaskExecution {
                        operation: "poll task status".to_string(),
                        details: e.to_string(),
                    })?;

            match response {
                A2aResponse::Success { result, .. } => {
                    let task_response: TaskGetResponse =
                        serde_json::from_value(result).map_err(|e| Error::TaskExecution {
                            operation: "parse task status response".to_string(),
                            details: e.to_string(),
                        })?;

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
                    return Err(Error::TaskExecution {
                        operation: "poll task status".to_string(),
                        details: format!("RPC error {}: {}", error.code, error.message),
                    });
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
            return Err(Error::Discovery {
                operation: "discover mesh agents".to_string(),
                details: "no mesh agents found via mDNS".to_string(),
            });
        }

        // Display discovered agents
        ui.section("Discovered Mesh Agents");
        for agent in &agents {
            ui.status(&format!("  - {} ({})", agent.name, agent.agent_id));
            if !agent.purpose.is_empty() {
                ui.status(&format!("    Purpose: {}", agent.purpose));
            }
            if !agent.capabilities.is_empty() {
                ui.status(&format!(
                    "    Capabilities: {}",
                    agent.capabilities.join(", ")
                ));
            }
        }

        // Select agent based on task content
        let agent = self.select_agent(task, &agents, config).await?;

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
