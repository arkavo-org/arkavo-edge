use crate::agent_connection::AgentConnection;
use crate::gateway_routing::{broadcast_event, build_routing_candidates, update_task_status};
use crate::types::*;
use arkavo_router::learning::LearningModule;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

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
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
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
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("AG-UI: Received RequestMeshStatus");

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

    let mesh_status = AgUiEvent::MeshStatus {
        agents: mesh_agents,
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    if let Ok(json) = serde_json::to_string(&mesh_status) {
        println!("AG-UI: MeshStatus JSON: {}", &json[..json.len().min(500)]);
    }
    tx.send(mesh_status).await?;
    println!("AG-UI: Sent MeshStatus with {} agents", agents_list.len());
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
            status: t.status.clone(),
            target_agent: t.target_agent.clone(),
            created_at: t.created_at.clone(),
            completed_at: t.completed_at.clone(),
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

#[allow(clippy::too_many_arguments)]
pub async fn handle_submit_task(
    description: String,
    target_agent: Option<String>,
    agents: &Arc<RwLock<Vec<serde_json::Value>>>,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    task_store: &Arc<RwLock<HashMap<String, super::gateway::TrackedTask>>>,
    connections: &Arc<RwLock<HashMap<String, super::gateway::ConnectionInfo>>>,
    learning_module: &Arc<RwLock<LearningModule>>,
    routing_history: &Arc<RwLock<VecDeque<RoutingRecord>>>,
    lesson_tx: &Option<mpsc::Sender<arkavo_router::learning::Lesson>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("AG-UI: Received SubmitTask: {}", description);

    let task_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    // Classify task by keywords for per-category routing
    let task_category = arkavo_router::classify_task_keywords(&description);
    let category_str = task_category.as_str().to_string();

    // Collect agent IDs for Thompson Sampling
    let agent_ids: Vec<String> = {
        let agents_list = agents.read().await;
        agents_list
            .iter()
            .filter_map(|a| a.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .collect()
    };

    // Use Thompson Sampling to select agent (unless explicit target)
    let (selected_agent, was_exploration) = if target_agent.is_some() {
        (target_agent.clone(), false)
    } else if agent_ids.is_empty() {
        (None, false)
    } else {
        let lm = learning_module.read().await;
        match lm
            .select_with_probation_guarantee(&agent_ids, Some(&category_str))
            .await
        {
            Some((agent_id, _score, was_prob)) => (Some(agent_id), was_prob),
            None => (select_target_agent(&None, agents).await, false),
        }
    };

    let agent_name = selected_agent.as_deref().unwrap_or("unassigned");
    println!(
        "AG-UI: Task {} targeting agent: {} (exploration={})",
        &task_id[..8],
        agent_name,
        was_exploration
    );

    // Build routing candidates for all agents (category-aware sampling)
    let candidates =
        build_routing_candidates(&agent_ids, learning_module, Some(&category_str)).await;

    // Emit RoutingEvaluation to all connected UIs
    if let Some(ref sel) = selected_agent {
        let eval_event = AgUiEvent::RoutingEvaluation {
            task_id: task_id.clone(),
            task_description: description.clone(),
            candidates,
            selected_agent: sel.clone(),
            was_exploration,
            category: Some(category_str.clone()),
            timestamp: now.clone(),
        };
        broadcast_event(&eval_event, connections).await;
    }

    // Store routing record
    {
        let record = RoutingRecord {
            task_id: task_id.clone(),
            selected_agent: agent_name.to_string(),
            was_exploration,
            outcome: None,
            quality_score: None,
            quality_issues: vec![],
            category: Some(category_str.clone()),
            timestamp: now.clone(),
        };
        let mut history = routing_history.write().await;
        history.push_back(record);
        while history.len() > 50 {
            history.pop_front();
        }
    }

    // Store the task
    let tracked = super::gateway::TrackedTask {
        id: task_id.clone(),
        description: description.clone(),
        target_agent: selected_agent.clone(),
        status: "submitted".to_string(),
        progress: Some(0.0),
        result: None,
        created_at: now.clone(),
        completed_at: None,
        task_category: Some(category_str),
    };
    task_store.write().await.insert(task_id.clone(), tracked);

    // Notify UI of submission
    tx.send(AgUiEvent::TaskSubmitted {
        task_id: task_id.clone(),
        status: "submitted".to_string(),
        timestamp: now.clone(),
    })
    .await?;

    // Forward task to agent via A2A message/send
    if let Some(ref agent_id) = selected_agent {
        let ac = agent_connections.read().await;
        if let Some(conn) = ac.get(agent_id) {
            let request = serde_json::json!({
                "message": {
                    "parts": [{ "type": "text", "content": description }]
                },
                "task_id": task_id
            });

            let task_store_clone = task_store.clone();
            let task_id_clone = task_id.clone();
            let connections_clone = connections.clone();
            let conn_clone = conn.clone();
            let agent_id_clone = agent_id.clone();
            let learning_clone = learning_module.clone();
            let history_clone = routing_history.clone();
            let lesson_tx_clone = lesson_tx.clone();

            tokio::spawn(async move {
                // Update status to working
                update_task_status(
                    &task_store_clone,
                    &task_id_clone,
                    "working",
                    Some(0.1),
                    None,
                    &connections_clone,
                    &learning_clone,
                    &history_clone,
                    &lesson_tx_clone,
                )
                .await;

                match conn_clone
                    .send_request("message/send", request, &task_id_clone)
                    .await
                {
                    Ok(response) => {
                        let result_text = response
                            .get("response")
                            .and_then(|r| r.get("parts"))
                            .and_then(|p| p.as_array())
                            .and_then(|arr| arr.first())
                            .and_then(|p| p.get("content"))
                            .and_then(|c| c.as_str())
                            .unwrap_or("Task completed")
                            .to_string();

                        println!(
                            "AG-UI: Task {} completed by {} ({} chars)",
                            &task_id_clone[..8],
                            agent_id_clone,
                            result_text.len()
                        );

                        update_task_status(
                            &task_store_clone,
                            &task_id_clone,
                            "completed",
                            Some(1.0),
                            Some(result_text),
                            &connections_clone,
                            &learning_clone,
                            &history_clone,
                            &lesson_tx_clone,
                        )
                        .await;
                    }
                    Err(e) => {
                        eprintln!(
                            "AG-UI: Task {} failed on {}: {}",
                            &task_id_clone[..8],
                            agent_id_clone,
                            e
                        );
                        update_task_status(
                            &task_store_clone,
                            &task_id_clone,
                            "failed",
                            None,
                            Some(format!("Error: {e}")),
                            &connections_clone,
                            &learning_clone,
                            &history_clone,
                            &lesson_tx_clone,
                        )
                        .await;
                    }
                }
            });
        }
    }

    Ok(())
}

async fn select_target_agent(
    target_agent: &Option<String>,
    agents: &Arc<RwLock<Vec<serde_json::Value>>>,
) -> Option<String> {
    // Use explicit target if provided
    if let Some(target) = target_agent {
        return Some(target.clone());
    }

    let agents_list = agents.read().await;

    // Prefer orchestrator
    if let Some(orch) = agents_list.iter().find(|a| {
        a.get("purpose")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase()
            .contains("orchestrat")
    }) {
        return orch
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
    }

    // Fall back to first agent
    agents_list
        .first()
        .and_then(|a| a.get("id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}
