use crate::gateway::ConnectionInfo;
use crate::types::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub fn spawn_status_broadcaster(connections: Arc<RwLock<HashMap<String, ConnectionInfo>>>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            use arkavo_mcp_tools::registry::ToolRegistry;
            use arkavo_memory::MemoryStorage;
            use arkavo_observability::health_reporter::HealthRegistry;
            use arkavo_router::Router as LlmRouter;

            let storage = match MemoryStorage::new().await {
                Ok(s) => Arc::new(s),
                Err(e) => {
                    eprintln!("Failed to init storage for health: {e}");
                    continue;
                }
            };
            let registry = ToolRegistry::new(storage);
            let tools = registry.list_tools();
            let browser_tool = tools.iter().find(|t| t.name.contains("browser"));
            let health_registry = HealthRegistry::global();
            let health_reports = health_registry.check_all().await;
            let overall_health = health_registry.get_overall_status().await;
            let conns = connections.read().await;

            let system_status = SystemStatus {
                uptime: format!(
                    "{} seconds",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                ),
                memory_usage: "N/A".to_string(),
                active_connections: conns.len() as u32,
            };
            let mcp_tools_status = McpToolsStatus {
                browser_available: browser_tool.is_some(),
                tools_count: tools.len(),
                last_used: None,
            };

            if let Ok(router) = LlmRouter::new().await {
                let llms = router
                    .get_available_llms()
                    .into_iter()
                    .map(|info| LlmStatus {
                        name: info.name,
                        provider: info.provider,
                        connected: info.available,
                        model: info.model,
                        requests_today: 0,
                    })
                    .collect();

                let health_data = HealthData {
                    status: format!("{:?}", overall_health),
                    components: health_reports
                        .iter()
                        .map(|r| ComponentHealth {
                            component: r.component.clone(),
                            status: format!("{:?}", r.status),
                            message: r.message.clone(),
                        })
                        .collect(),
                };

                let status_event = AgUiEvent::StatusUpdate {
                    system: system_status,
                    mcp_tools: mcp_tools_status,
                    llms,
                    health: health_data,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                };
                for (_, conn_info) in conns.iter() {
                    let _ = conn_info._ws_tx.send(status_event.clone()).await;
                }
            }
        }
    });
}

pub fn spawn_agent_monitor(
    connections: Arc<RwLock<HashMap<String, ConnectionInfo>>>,
    agents: Arc<RwLock<Vec<serde_json::Value>>>,
) {
    tokio::spawn(async move {
        let mut previous: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
        loop {
            interval.tick().await;
            let agents_list = agents.read().await;
            let current: std::collections::HashSet<String> = agents_list
                .iter()
                .filter_map(|a| a.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                .collect();

            for agent_id in current.difference(&previous) {
                if let Some(agent) = agents_list
                    .iter()
                    .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(agent_id.as_str()))
                {
                    let event = AgUiEvent::AgentDiscovered {
                        agent_id: agent_id.clone(),
                        endpoint: agent
                            .get("endpoint")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        purpose: agent
                            .get("purpose")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        model: agent
                            .get("model")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        timestamp: chrono::Utc::now().to_rfc3339(),
                    };
                    let conns = connections.read().await;
                    for (_, ci) in conns.iter() {
                        let _ = ci._ws_tx.send(event.clone()).await;
                    }
                }
            }

            for agent_id in previous.difference(&current) {
                let event = AgUiEvent::AgentLost {
                    agent_id: agent_id.clone(),
                    reason: "Agent no longer discovered".to_string(),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                };
                let conns = connections.read().await;
                for (_, ci) in conns.iter() {
                    let _ = ci._ws_tx.send(event.clone()).await;
                }
            }

            previous = current;
        }
    });
}
