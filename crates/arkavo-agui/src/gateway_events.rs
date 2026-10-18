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
    tracing::debug!("AG-UI: Received SubmitPrompt: {text}");

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

    tracing::debug!(
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
    tracing::debug!("AG-UI: Received RequestStatus");
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

    tracing::debug!(
        "AG-UI: Health Status - Overall: {:?}, Components: {}",
        overall_health,
        health_reports.len()
    );
    Ok(())
}

/// Fleet polling must not wait for remote key-service requests.
pub async fn handle_request_fleet_snapshot(
    agents: &Arc<RwLock<Vec<serde_json::Value>>>,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<crate::agent_connection::AgentConnection>>>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let agents_list = agents.read().await;
    let mut mesh_agents: Vec<MeshAgentInfo> = agents_list
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
            status: "disconnected".to_string(),
        })
        .collect();
    drop(agents_list);
    let connections = agent_connections.read().await.clone();
    for agent in &mut mesh_agents {
        if let Some(connection) = connections.get(&agent.id) {
            agent.status = match connection.get_status().await {
                crate::agent_connection::ConnectionStatus::Connected => "connected",
                // An agent that has never answered is not reconnecting.
                crate::agent_connection::ConnectionStatus::Connecting => "connecting",
                crate::agent_connection::ConnectionStatus::Reconnecting { .. } => "reconnecting",
                crate::agent_connection::ConnectionStatus::Failed { .. } => "disconnected",
            }
            .to_string();
        }
    }

    let mesh_status = AgUiEvent::MeshStatus {
        agents: mesh_agents,
        timestamp: chrono::Utc::now().to_rfc3339(),
    };
    tx.send(mesh_status).await?;

    Ok(())
}

pub async fn handle_request_mesh_status(
    agents: &Arc<RwLock<Vec<serde_json::Value>>>,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<crate::agent_connection::AgentConnection>>>>,
    security_handler: &Arc<RwLock<crate::security_handler::SecurityHandler>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    handle_request_fleet_snapshot(agents, agent_connections, tx).await?;

    // Metrics (RSS, CPU, compute budget, Iroh) are now push-based via
    // system.metrics.subscribe — no polling needed. Only KAS status is still polled
    // (lightweight, infrequent, separate auth concern).
    let conns = agent_connections.read().await;
    for (agent_id, conn) in conns.iter() {
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
    tracing::debug!("AG-UI: Received ApplyPart: {part_id}");

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
                tracing::debug!(
                    "AG-UI: Stream complete for part {} after {} chunks",
                    part_id_clone,
                    chunk_count
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
    tracing::debug!("AG-UI: Received RequestTaskList");

    let store = task_store.read().await;
    let tasks: Vec<TaskInfo> = store
        .values()
        .map(|t| TaskInfo {
            id: t.id.clone(),
            description: t.description.clone(),
            summary: t.summary.clone(),
            has_result: t.result.is_some(),
            assets: t.assets.iter().map(|stored| stored.asset.clone()).collect(),
            status: t.status.clone(),
            target_agent: t.target_agent.clone(),
            created_at: t.created_at.clone(),
            completed_at: t.completed_at.clone(),
            source: Some(t.source.clone()),
        })
        .collect();

    tracing::debug!("AG-UI: Sending TaskList with {} tasks", tasks.len());
    tx.send(AgUiEvent::TaskList {
        tasks,
        timestamp: chrono::Utc::now().to_rfc3339(),
    })
    .await?;
    tracing::debug!("AG-UI: Sent TaskList");
    Ok(())
}

/// One task's output for a view that opened the task without it: after a
/// reload, or for a task another view submitted.
pub async fn handle_request_task(
    task_id: &str,
    task_store: &Arc<RwLock<HashMap<String, super::gateway::TrackedTask>>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(task) = task_store.read().await.get(task_id).cloned() else {
        return Ok(());
    };
    tx.send(AgUiEvent::TaskDetail {
        task_id: task.id,
        result: task.result,
        assets: task.assets.into_iter().map(|stored| stored.asset).collect(),
        asset_note: task.asset_note,
    })
    .await?;
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "Tokio test harness runs outside an async runtime"
)]
mod tests {
    use super::*;

    /// AGUI-024: the task list, sent every few seconds to every view, says
    /// a task has output without carrying it; a view that opens the task
    /// asks for the whole of it.
    #[arkavo_test_macros::spec("AGUI-024")]
    #[tokio::test]
    async fn a_fresh_view_gets_the_complete_output_on_request() {
        let output = "Observation — 未確認\n".repeat(200);
        let task = crate::gateway::TrackedTask {
            id: "triage-1".into(),
            description: "Assess supplied observations".into(),
            summary: Some("Triage observations".into()),
            result: Some(output.clone()),
            target_agent: Some("triage".into()),
            status: "completed".into(),
            progress: Some(1.0),
            created_at: "2026-10-17T12:00:00Z".into(),
            completed_at: Some("2026-10-17T12:00:10Z".into()),
            task_category: None,
            first_working_at: None,
            source: "ui".into(),
            request_id: None,
            assets: Vec::new(),
            asset_note: None,
        };
        let store = Arc::new(RwLock::new(HashMap::from([(task.id.clone(), task)])));
        let (tx, mut rx) = mpsc::channel(1);
        handle_request_task_list(&store, &tx).await.unwrap();
        let list = serde_json::to_value(rx.recv().await.unwrap()).unwrap();
        assert_eq!(list["tasks"][0]["hasResult"], true);
        assert!(list["tasks"][0].get("result").is_none(), "{list}");
        assert_eq!(list["tasks"][0]["summary"], "Triage observations");

        handle_request_task("triage-1", &store, &tx).await.unwrap();
        let detail = serde_json::to_value(rx.recv().await.unwrap()).unwrap();
        assert_eq!(detail["type"], "taskDetail");
        assert_eq!(detail["result"], output);
    }

    #[arkavo_test_macros::spec("AGUI-003")]
    #[tokio::test]
    async fn discovery_does_not_imply_a_connected_agent() {
        let agents = Arc::new(RwLock::new(vec![
            serde_json::json!({"id": "discovered"}),
            serde_json::json!({"id": "connecting"}),
        ]));
        let (telemetry_tx, _telemetry_rx) = mpsc::channel(8);
        let connection = crate::agent_connection::AgentConnection::new(
            "connecting".into(),
            "ws://127.0.0.1:1".into(),
            telemetry_tx,
        );
        let connections = Arc::new(RwLock::new(HashMap::from([(
            "connecting".into(),
            Arc::new(connection),
        )])));
        let (tx, mut rx) = mpsc::channel(8);
        handle_request_fleet_snapshot(&agents, &connections, &tx)
            .await
            .unwrap();
        let AgUiEvent::MeshStatus { agents, .. } = rx.recv().await.unwrap() else {
            panic!("expected fleet snapshot");
        };
        assert_eq!(agents[0].status, "disconnected");
        assert_eq!(agents[1].status, "connecting");
    }
}
