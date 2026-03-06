use crate::agent_connection::AgentConnection;
use crate::gateway_events::short_id;
use crate::gateway_routing::{broadcast_event, build_routing_candidates, update_task_status};
use crate::types::*;
use arkavo_router::learning::LearningModule;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

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
    lesson_store: &Arc<RwLock<Vec<arkavo_router::learning::Lesson>>>,
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
        short_id(&task_id),
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
        first_working_at: None,
        source: "ui".to_string(),
        summary: None,
    };
    task_store.write().await.insert(task_id.clone(), tracked);

    // Notify UI of submission
    tx.send(AgUiEvent::TaskSubmitted {
        task_id: task_id.clone(),
        description: Some(description.clone()),
        summary: None,
        status: "submitted".to_string(),
        source: Some("ui".to_string()),
        timestamp: now.clone(),
    })
    .await?;

    // Forward task to agent via A2A message/send
    if let Some(ref agent_id) = selected_agent {
        let ac = agent_connections.read().await;
        if let Some(conn) = ac.get(agent_id) {
            // Inject behavior guidance from cached lessons
            let augmented_description =
                augment_with_lessons(&description, &task_id, task_store, lesson_store).await;

            let request = serde_json::json!({
                "message": {
                    "parts": [{ "type": "text", "content": augmented_description }]
                },
                "task_id": task_id
            });

            // Look up agent's declared model for ModelSelected event
            let agent_model = {
                let agents_list = agents.read().await;
                agents_list
                    .iter()
                    .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(agent_id))
                    .and_then(|a| a.get("model").and_then(|v| v.as_str()))
                    .unwrap_or("unknown")
                    .to_string()
            };

            let task_store_clone = task_store.clone();
            let task_id_clone = task_id.clone();
            let connections_clone = connections.clone();
            let conn_clone = conn.clone();
            let agent_id_clone = agent_id.clone();
            let learning_clone = learning_module.clone();
            let history_clone = routing_history.clone();
            let lesson_tx_clone = lesson_tx.clone();

            tokio::spawn(async move {
                poll_agent_task(
                    conn_clone,
                    request,
                    &task_id_clone,
                    &agent_id_clone,
                    &agent_model,
                    &task_store_clone,
                    &connections_clone,
                    &learning_clone,
                    &history_clone,
                    &lesson_tx_clone,
                )
                .await;
            });
        }
    }

    Ok(())
}

async fn augment_with_lessons(
    description: &str,
    task_id: &str,
    task_store: &Arc<RwLock<HashMap<String, super::gateway::TrackedTask>>>,
    lesson_store: &Arc<RwLock<Vec<arkavo_router::learning::Lesson>>>,
) -> String {
    let store = lesson_store.read().await;
    let category_str_ref = task_store
        .read()
        .await
        .get(task_id)
        .and_then(|t| t.task_category.clone());
    let cat = category_str_ref.as_deref().unwrap_or("general");
    let relevant: Vec<&str> = store
        .iter()
        .filter(|l| l.category == cat || l.category == "general")
        .map(|l| l.pattern.condition.as_str())
        .collect();
    if relevant.is_empty() {
        return description.to_string();
    }
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<&str> = relevant
        .into_iter()
        .filter(|c| seen.insert(*c))
        .take(5)
        .collect();
    let guidance = format!(
        "## Prior Lessons Learned\n{}\n\n## Task\n{}",
        unique
            .iter()
            .map(|c| format!("- Avoid: {c}"))
            .collect::<Vec<_>>()
            .join("\n"),
        description
    );
    println!(
        "AG-UI: Injecting {} chars of behavior guidance ({} lessons)",
        guidance.len() - description.len(),
        unique.len()
    );
    guidance
}

#[allow(clippy::too_many_arguments)]
async fn poll_agent_task(
    conn: Arc<AgentConnection>,
    request: serde_json::Value,
    task_id: &str,
    agent_id: &str,
    agent_model: &str,
    task_store: &Arc<RwLock<HashMap<String, super::gateway::TrackedTask>>>,
    connections: &Arc<RwLock<HashMap<String, super::gateway::ConnectionInfo>>>,
    learning_module: &Arc<RwLock<LearningModule>>,
    routing_history: &Arc<RwLock<VecDeque<RoutingRecord>>>,
    lesson_tx: &Option<mpsc::Sender<arkavo_router::learning::Lesson>>,
) {
    // Submit task to agent — returns immediately with task_id
    let agent_task_id = match conn.send_request("message/send", request, task_id).await {
        Ok(response) => {
            let tid = response
                .get("task_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            println!(
                "AG-UI: Task {} submitted to {}, agent_task_id={}",
                short_id(task_id),
                agent_id,
                &tid[..tid.len().min(8)]
            );
            tid
        }
        Err(e) => {
            eprintln!(
                "AG-UI: Task {} submit failed on {}: {}",
                short_id(task_id),
                agent_id,
                e
            );
            update_task_status(
                task_store,
                task_id,
                "failed",
                None,
                Some(format!("Error: {e}")),
                connections,
                learning_module,
                routing_history,
                lesson_tx,
            )
            .await;
            return;
        }
    };

    // Update status to working
    update_task_status(
        task_store,
        task_id,
        "working",
        Some(0.1),
        None,
        connections,
        learning_module,
        routing_history,
        lesson_tx,
    )
    .await;

    // Poll tasks/get until terminal status
    let mut progress = 0.15_f32;
    let poll_interval = std::time::Duration::from_secs(2);
    let max_polls = 150; // 5 minutes max
    for _ in 0..max_polls {
        tokio::time::sleep(poll_interval).await;

        let poll_request = serde_json::json!({ "task_id": agent_task_id });
        match conn.send_request("tasks/get", poll_request, task_id).await {
            Ok(poll_response) => {
                let status = poll_response
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                match status {
                    "Completed" | "completed" => {
                        let result_text = poll_response
                            .get("result")
                            .and_then(|r| r.get("parts"))
                            .and_then(|p| p.as_array())
                            .and_then(|arr| arr.first())
                            .and_then(|p| p.get("content"))
                            .and_then(|c| c.as_str())
                            .unwrap_or("Task completed")
                            .to_string();

                        println!(
                            "AG-UI: Task {} completed by {} ({} chars)",
                            short_id(task_id),
                            agent_id,
                            result_text.len()
                        );

                        let model_event = AgUiEvent::ModelSelected {
                            agent_id: agent_id.to_string(),
                            provider: "local".to_string(),
                            model: agent_model.to_string(),
                            estimated_cost: arkavo_budget::TokenCost::ZERO,
                            reason: format!("Agent {} selected via Thompson Sampling", agent_id),
                            event_id: uuid::Uuid::new_v4().to_string(),
                        };
                        broadcast_event(&model_event, connections).await;

                        update_task_status(
                            task_store,
                            task_id,
                            "completed",
                            Some(1.0),
                            Some(result_text),
                            connections,
                            learning_module,
                            routing_history,
                            lesson_tx,
                        )
                        .await;
                        return;
                    }
                    "Failed" | "failed" => {
                        let error_msg = poll_response
                            .get("error")
                            .and_then(|e| e.get("message"))
                            .and_then(|m| m.as_str())
                            .unwrap_or("Task failed")
                            .to_string();

                        eprintln!(
                            "AG-UI: Task {} failed on {}: {}",
                            short_id(task_id),
                            agent_id,
                            error_msg
                        );
                        update_task_status(
                            task_store,
                            task_id,
                            "failed",
                            None,
                            Some(error_msg),
                            connections,
                            learning_module,
                            routing_history,
                            lesson_tx,
                        )
                        .await;
                        return;
                    }
                    "Canceled" | "canceled" | "Rejected" | "rejected" => {
                        update_task_status(
                            task_store,
                            task_id,
                            status,
                            None,
                            None,
                            connections,
                            learning_module,
                            routing_history,
                            lesson_tx,
                        )
                        .await;
                        return;
                    }
                    _ => {
                        // Still working — update progress
                        if progress < 0.9 {
                            progress += 0.05;
                        }
                        let event = AgUiEvent::TaskStatusChanged {
                            task_id: task_id.to_string(),
                            status: "working".to_string(),
                            progress: Some(progress),
                            result: None,
                            metrics: None,
                            timestamp: chrono::Utc::now().to_rfc3339(),
                        };
                        broadcast_event(&event, connections).await;
                    }
                }
            }
            Err(e) => {
                eprintln!("AG-UI: Task {} poll error: {}", short_id(task_id), e);
            }
        }
    }

    // Timeout after max polls
    eprintln!(
        "AG-UI: Task {} timed out after 5 minutes",
        short_id(task_id)
    );
    update_task_status(
        task_store,
        task_id,
        "failed",
        None,
        Some("Task timed out after 5 minutes".to_string()),
        connections,
        learning_module,
        routing_history,
        lesson_tx,
    )
    .await;
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
