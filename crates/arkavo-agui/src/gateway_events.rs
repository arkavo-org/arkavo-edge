use crate::types::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

pub(crate) fn short_id(id: &str) -> &str {
    &id[..id.len().min(8)]
}

pub async fn handle_submit_prompt(
    text: String,
    session_id: &str,
    connections: &Arc<RwLock<HashMap<String, super::gateway::ConnectionInfo>>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("AG-UI: Received SubmitPrompt: {text}");

    let (cleaned_text, api_keys) = crate::api_keys::extract_api_keys(&text);
    if !api_keys.is_empty() {
        crate::api_keys::set_api_keys(&api_keys);
        send_status_update(connections, tx).await?;
    }

    use arkavo_router::Router;
    use arkavo_ui_generator::planner::UiPlanner;

    let router = Arc::new(Router::new().await?);
    let planner = UiPlanner::new(router);
    let plan = planner.plan(&cleaned_text).await?;

    let mut conn_guard = connections.write().await;
    if let Some(conn_info) = conn_guard.get_mut(session_id) {
        conn_info.current_plan = Some(plan.parts.clone());
        conn_info.current_prompt = Some(cleaned_text.clone());
    }
    drop(conn_guard);

    let parts: Vec<UiPlanPart> = plan
        .parts
        .iter()
        .map(|p| UiPlanPart {
            id: p.id.clone(),
            name: p.name.clone(),
            description: p.description.clone(),
        })
        .collect();

    println!(
        "AG-UI: Sending Plan event with {} parts to frontend",
        parts.len()
    );
    tx.send(AgUiEvent::Plan { parts }).await?;
    Ok(())
}

pub async fn handle_request_status(
    connections: &Arc<RwLock<HashMap<String, super::gateway::ConnectionInfo>>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("AG-UI: Received RequestStatus");
    send_status_update(connections, tx).await
}

async fn send_status_update(
    connections: &Arc<RwLock<HashMap<String, super::gateway::ConnectionInfo>>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    use arkavo_mcp_tools::registry::ToolRegistry;
    use arkavo_memory::MemoryStorage;
    use arkavo_observability::health_reporter::HealthRegistry;
    use arkavo_router::Router;

    let storage = Arc::new(MemoryStorage::new().await?);
    let registry = ToolRegistry::new(storage);
    let tools = registry.list_tools();
    let browser_tool = tools.iter().find(|t| t.name.contains("browser"));

    let health_registry = HealthRegistry::global();
    let health_reports = health_registry.check_all().await;
    let overall_health = health_registry.get_overall_status().await;

    let system_status = SystemStatus {
        uptime: format!(
            "{} seconds",
            crate::types::PROCESS_START.elapsed().as_secs()
        ),
        memory_usage: "N/A".to_string(),
        active_connections: connections.read().await.len() as u32,
    };

    let mcp_tools_status = McpToolsStatus {
        browser_available: browser_tool.is_some(),
        tools_count: tools.len(),
        last_used: None,
    };

    let router = Router::new().await?;
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

    tx.send(status_event).await?;

    println!(
        "AG-UI: Health Status - Overall: {:?}, Components: {}",
        overall_health,
        health_reports.len()
    );
    Ok(())
}

pub async fn handle_request_mesh_status(
    agents: &Arc<RwLock<Vec<serde_json::Value>>>,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<crate::agent_connection::AgentConnection>>>>,
    security_handler: &Arc<RwLock<crate::security_handler::SecurityHandler>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let agents_list = agents.read().await;
    let mesh_agents: Vec<MeshAgentInfo> = agents_list
        .iter()
        .map(|a| MeshAgentInfo {
            id: a
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            endpoint: a
                .get("endpoint")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            purpose: a
                .get("purpose")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            model: a
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            status: "connected".to_string(),
        })
        .collect();
    drop(agents_list);

    let mesh_status = AgUiEvent::MeshStatus {
        agents: mesh_agents,
        timestamp: chrono::Utc::now().to_rfc3339(),
    };
    tx.send(mesh_status).await?;

    // Poll compute budget and system metrics from each connected agent via JSON-RPC
    let conns = agent_connections.read().await;
    for (agent_id, conn) in conns.iter() {
        if let Ok(budget_data) = conn.get_compute_budget().await {
            let event = AgUiEvent::ComputeBudgetUpdate {
                agent_id: agent_id.clone(),
                compute_budget: budget_data,
                timestamp: chrono::Utc::now().to_rfc3339(),
            };
            let _ = tx.send(event).await;
        }

        // Poll per-agent process metrics (RSS, CPU, subsystem timing)
        if let Ok(metrics_data) = conn.get_system_metrics().await {
            let event = AgUiEvent::AgentSystemMetrics {
                agent_id: agent_id.clone(),
                rss_mb: metrics_data["rss_mb"].as_f64().unwrap_or(0.0),
                cpu_percent: metrics_data["cpu_percent"].as_f64().unwrap_or(0.0),
                pid: metrics_data["pid"].as_u64().unwrap_or(0) as u32,
                total_ram_mb: metrics_data["total_ram_mb"].as_f64(),
                available_ram_mb: metrics_data["available_ram_mb"].as_f64(),
            };
            let _ = tx.send(event).await;

            // Forward agent subsystem timing into the gateway's global registry
            // so the MetricsSampler includes it in telemetry broadcasts.
            if let Some(timing) = metrics_data.get("subsystem_timing") {
                let registry = arkavo_observability::subsystem_timing::global_timing();
                if let Some(ms) = timing.get("routerDecisionAvgMs").and_then(|v| v.as_f64())
                    && ms > 0.0
                {
                    registry.router_decisions.record(ms as u64);
                }
                if let Some(ms) = timing
                    .get("conductorOrchestrationAvgMs")
                    .and_then(|v| v.as_f64())
                    && ms > 0.0
                {
                    registry.conductor_orchestration.record(ms as u64);
                }
                if let Some(ms) = timing.get("mcpToolAvgMs").and_then(|v| v.as_f64())
                    && ms > 0.0
                {
                    registry.mcp_tools.record(ms as u64);
                }
                if let Some(ms) = timing.get("inferenceAvgMs").and_then(|v| v.as_f64())
                    && ms > 0.0
                {
                    registry.inference.record(ms as u64);
                }
            }
        }

        // Poll KAS public key to detect KAS-enabled agents
        // Perform network I/O before acquiring lock to avoid holding it during RPC
        let kas_result = conn.get_kas_public_key().await;
        let sec = security_handler.read().await;
        match kas_result {
            Ok(kas_data) => {
                let info = crate::security_handler::AgentKasInfo {
                    enabled: true,
                    public_key: kas_data["public_key"].as_str().unwrap_or("").to_string(),
                    key_id: kas_data["key_id"].as_str().unwrap_or("").to_string(),
                    algorithm: kas_data["algorithm"].as_str().unwrap_or("").to_string(),
                };
                sec.update_agent_kas(agent_id, info).await;
            }
            Err(_) => {
                sec.mark_agent_no_kas(agent_id).await;
            }
        }
        drop(sec);
    }

    Ok(())
}

pub async fn handle_apply_part(
    part_id: String,
    session_id: &str,
    connections: &Arc<RwLock<HashMap<String, super::gateway::ConnectionInfo>>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("AG-UI: Received ApplyPart: {part_id}");

    let conn_guard = connections.read().await;
    let conn_info = conn_guard.get(session_id);

    let (part_name, part_description, overall_prompt) = if let Some(info) = conn_info {
        let plan = info.current_plan.as_ref();
        let prompt = info.current_prompt.as_ref();
        let part = plan.and_then(|p| p.iter().find(|part| part.id == part_id));
        (
            part.map(|p| p.name.clone())
                .unwrap_or_else(|| "Component".to_string()),
            part.map(|p| p.description.clone())
                .unwrap_or_else(|| "UI Component".to_string()),
            prompt
                .cloned()
                .unwrap_or_else(|| "Build a modern web component".to_string()),
        )
    } else {
        (
            "Component".to_string(),
            "UI Component".to_string(),
            "Build a modern web component".to_string(),
        )
    };
    drop(conn_guard);

    use arkavo_router::Router;
    use arkavo_ui_generator::streaming::StreamingGenerator;

    let router = Arc::new(Router::new().await?);
    let generator = StreamingGenerator::new(router)?;
    let mut stream_rx = generator
        .generate_part(&part_name, &part_description, &overall_prompt)
        .await?;

    let tx_clone = tx.clone();
    let part_id_clone = part_id.clone();
    tokio::spawn(async move {
        let mut chunk_count = 0;
        while let Some(chunk) = stream_rx.recv().await {
            chunk_count += 1;

            let stream_event = AgUiEvent::PartStream {
                part_id: part_id_clone.clone(),
                chunk_type: match chunk.chunk_type {
                    arkavo_ui_generator::streaming::ChunkType::Html => "html".to_string(),
                    arkavo_ui_generator::streaming::ChunkType::Css => "css".to_string(),
                    arkavo_ui_generator::streaming::ChunkType::JavaScript => "js".to_string(),
                },
                content: chunk.content,
                done: chunk.done,
            };

            if tx_clone.send(stream_event).await.is_err() {
                break;
            }

            if chunk.done {
                println!(
                    "AG-UI: Stream complete for part {} after {} chunks",
                    part_id_clone, chunk_count
                );
                let applied_event = AgUiEvent::AppliedPart {
                    part_id: part_id_clone.clone(),
                    version_id: uuid::Uuid::new_v4().to_string(),
                };
                let _ = tx_clone.send(applied_event).await;
                break;
            }
        }
    });

    Ok(())
}

pub async fn handle_request_task_list(
    task_store: &Arc<RwLock<HashMap<String, super::gateway::TrackedTask>>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("AG-UI: Received RequestTaskList");

    let store = task_store.read().await;
    let tasks: Vec<TaskInfo> = store
        .values()
        .map(|t| TaskInfo {
            id: t.id.clone(),
            description: t.description.clone(),
            summary: t.summary.clone(),
            status: t.status.clone(),
            target_agent: t.target_agent.clone(),
            created_at: t.created_at.clone(),
            completed_at: t.completed_at.clone(),
            source: Some(t.source.clone()),
        })
        .collect();

    println!("AG-UI: Sending TaskList with {} tasks", tasks.len());
    tx.send(AgUiEvent::TaskList {
        tasks,
        timestamp: chrono::Utc::now().to_rfc3339(),
    })
    .await?;
    println!("AG-UI: Sent TaskList");
    Ok(())
}
