use crate::agent_connection::AgentConnection;
use crate::arp_handler::ArpHandler;
use crate::gateway::ConnectionInfo;
use crate::types::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Poll interval for per-agent ARP documents. ARPs change on proposal
/// application or operator edits — far slower than task state.
const ARP_POLL_INTERVAL_SECS: u64 = 30;

/// Apply one agent's `arp/get` response to the ARP handler. Returns true
/// when a document was installed. Extracted from the poll loop for
/// testability.
pub(crate) async fn apply_arp_response(
    arp_handler: &ArpHandler,
    agent_id: &str,
    response: &serde_json::Value,
) -> bool {
    match response.get("document") {
        Some(doc) if !doc.is_null() => {
            arp_handler.set_agent_arp(agent_id, &doc.to_string()).await;
            true
        }
        _ => false,
    }
}

/// Per-agent A2A ARP polling: every interval, ask each connected agent for
/// its effective ARP document and install it under that agent's id. This is
/// what retires the synthetic `(local)` placeholder as the only key — the
/// gateway's own document keeps that id; mesh agents report their own.
pub fn spawn_arp_poller(
    agent_connections: Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    arp_handler: Arc<ArpHandler>,
) {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_secs(ARP_POLL_INTERVAL_SECS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let conns: Vec<(String, Arc<AgentConnection>)> = {
                let guard = agent_connections.read().await;
                guard
                    .iter()
                    .map(|(id, conn)| (id.clone(), conn.clone()))
                    .collect()
            };
            for (agent_id, conn) in conns {
                match conn
                    .send_request("arp/get", serde_json::json!({}), "arp-poll")
                    .await
                {
                    Ok(response) => {
                        apply_arp_response(&arp_handler, &agent_id, &response).await;
                    }
                    Err(e) => {
                        // Older agents don't expose arp/get; absence is not
                        // an error worth surfacing per poll.
                        tracing::debug!(agent_id = %agent_id, error = %e, "arp/get poll failed");
                    }
                }
            }
        }
    });
}

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
                    crate::types::PROCESS_START.elapsed().as_secs()
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
                // Router telemetry from the process-wide event counters (the
                // real serving router increments these at emit time, so this
                // needs no router handle and never double-drains).
                let router_telemetry = AgUiEvent::RouterTelemetry {
                    counters: arkavo_observability::event_counters::global_event_counters()
                        .snapshot(),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                };
                for (_, conn_info) in conns.iter() {
                    let _ = conn_info._ws_tx.send(status_event.clone()).await;
                    let _ = conn_info._ws_tx.send(router_telemetry.clone()).await;
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

#[cfg(test)]
mod tests {
    // #[tokio::test] expands to Runtime::block_on; harmless in tests.
    #![allow(clippy::disallowed_methods)]

    use super::*;

    const MIN_DOC: &str = r#"{
        "arp_spec": "0.1.0",
        "adl_ref": {"uri": "https://example.com/adl.json"},
        "adaptation": {"method": "thompson_sampling"},
        "feedback_loops": {
            "immediate": {
                "quality_gate": {
                    "threshold_default": 0.7,
                    "metric": "cosine_similarity",
                    "on_failure": "update_prior_and_log"
                }
            },
            "short_term": {
                "policy_cache": {
                    "default_ttl_sec": 3600,
                    "decay_strategy": "linear"
                }
            }
        },
        "budget": {
            "task_ceiling_usd": 2.5,
            "on_exhaustion": "halt_and_report",
            "velocity": {"max_spend_per_minute_usd": 0.5}
        }
    }"#;

    #[tokio::test]
    async fn apply_arp_response_installs_document_under_agent_id() {
        let handler = ArpHandler::new();
        let doc: serde_json::Value = serde_json::from_str(MIN_DOC).unwrap();
        let response = serde_json::json!({ "document": doc });

        assert!(apply_arp_response(&handler, "rover-alpha", &response).await);
        let snapshot = handler.snapshot().await;
        let agent = snapshot
            .agents
            .iter()
            .find(|a| a.agent_id == "rover-alpha")
            .expect("polled agent appears in the mesh snapshot");
        assert!(agent.document.is_some());
    }

    #[tokio::test]
    async fn apply_arp_response_ignores_null_or_missing_document() {
        let handler = ArpHandler::new();
        assert!(!apply_arp_response(&handler, "a", &serde_json::json!({ "document": null })).await);
        assert!(!apply_arp_response(&handler, "a", &serde_json::json!({})).await);
        assert!(handler.snapshot().await.agents.is_empty());
    }
}
