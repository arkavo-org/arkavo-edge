use crate::agent_connection::AgentConnection;
use crate::gateway_routing::broadcast_event;
use crate::types::AgUiEvent;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Poll connected agents for their internal HRM tasks and sync into the UI dashboard.
///
/// Emits TaskSubmitted/TaskStatusChanged/TelemetryEvent so browser sessions
/// see agent-internal work in real-time.
pub fn spawn_agent_task_sync(
    connections: Arc<RwLock<HashMap<String, super::gateway::ConnectionInfo>>>,
    agent_connections: Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    task_store: Arc<RwLock<HashMap<String, super::gateway::TrackedTask>>>,
) {
    tokio::spawn(async move {
        // Wait for agents to register before first poll
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;

        // Create router for task summarization (best-effort)
        let router = match arkavo_router::Router::new().await {
            Ok(r) => {
                println!("AG-UI: task summarizer ready");
                Some(Arc::new(r))
            }
            Err(e) => {
                eprintln!("AG-UI: task summarizer unavailable: {e}");
                None
            }
        };

        // Track known agent tasks: composite key → (status, progress)
        let mut known: HashMap<String, (String, f64)> = HashMap::new();

        loop {
            if let Err(e) = poll_agent_tasks(
                &connections,
                &agent_connections,
                &task_store,
                router.as_ref(),
                &mut known,
            )
            .await
            {
                eprintln!("AG-UI: task sync error: {e}");
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
}

async fn summarize_task(router: &arkavo_router::Router, objective: &str) -> Option<String> {
    if objective.len() < 80 {
        return None;
    }
    let truncated = &objective[..objective.len().min(2000)];
    let prompt = format!(
        "Summarize this agent task in one short sentence (max 15 words). \
         Only output the summary, nothing else.\n\n{truncated}"
    );
    let messages = vec![arkavo_llm::Message::user(prompt)];
    match router.route_chat(messages).await {
        Ok(resp) if !resp.content.is_empty() => {
            let summary = resp.content.trim().to_string();
            if summary.is_empty() {
                None
            } else {
                Some(summary)
            }
        }
        _ => None,
    }
}

async fn poll_agent_tasks(
    connections: &Arc<RwLock<HashMap<String, super::gateway::ConnectionInfo>>>,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    task_store: &Arc<RwLock<HashMap<String, super::gateway::TrackedTask>>>,
    router: Option<&Arc<arkavo_router::Router>>,
    known: &mut HashMap<String, (String, f64)>,
) -> Result<(), Box<dyn std::error::Error>> {
    let agents: Vec<(String, Arc<AgentConnection>)> = {
        let conns = agent_connections.read().await;
        conns
            .iter()
            .map(|(id, conn)| (id.clone(), conn.clone()))
            .collect()
    };

    for (agent_id, conn) in agents {
        let params = serde_json::json!({"limit": 20});
        let resp = match conn.send_request("tasks/list", params, "task-sync").await {
            Ok(r) => r,
            Err(_) => continue, // skip disconnected agents
        };

        let tasks = match resp.get("tasks").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => continue,
        };

        for task_val in tasks {
            let Some(task_id) = task_val.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let status = task_val
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("Pending");
            let objective = task_val
                .get("objective")
                .and_then(|v| v.as_str())
                .unwrap_or("Agent task");
            let created_at = task_val
                .get("created_at")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let progress = task_val
                .get("progress")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);

            let composite_key = format!("{}:{}", agent_id, task_id);
            let ui_status = hrm_status_to_ui(status);
            let quality_score = task_val.get("quality_score").and_then(|v| v.as_f64());

            match known.get(&composite_key) {
                None => {
                    // New task — insert and emit TaskSubmitted
                    known.insert(composite_key.clone(), (status.to_string(), progress));

                    let summary = if let Some(r) = router {
                        summarize_task(r, objective).await
                    } else {
                        None
                    };

                    let tracked = super::gateway::TrackedTask {
                        id: composite_key.clone(),
                        description: objective.to_string(),
                        target_agent: Some(agent_id.clone()),
                        status: ui_status.clone(),
                        progress: Some(progress as f32),
                        result: None,
                        created_at: created_at.to_string(),
                        completed_at: None,
                        task_category: None,
                        first_working_at: None,
                        source: "agent".to_string(),
                        summary: summary.clone(),
                    };
                    task_store
                        .write()
                        .await
                        .insert(composite_key.clone(), tracked);

                    let submitted = AgUiEvent::TaskSubmitted {
                        task_id: composite_key.clone(),
                        description: Some(objective.to_string()),
                        summary,
                        status: ui_status.clone(),
                        source: Some("agent".to_string()),
                        timestamp: created_at.to_string(),
                    };
                    broadcast_event(&submitted, connections).await;

                    // If the task already has quality data or non-zero progress,
                    // emit TaskStatusChanged so the UI gets metrics on first sync.
                    if quality_score.is_some() || progress > 0.0 {
                        let metrics = quality_score.map(|q| crate::types::TaskMetrics {
                            tokens_generated: 0,
                            tokens_per_sec: 0.0,
                            ttft_ms: 0,
                            inference_duration_ms: 0,
                            energy_wh: 0.0,
                            quality_score: Some(q),
                        });
                        let changed = AgUiEvent::TaskStatusChanged {
                            task_id: composite_key.clone(),
                            status: ui_status.clone(),
                            progress: Some(progress as f32),
                            result: None,
                            metrics,
                            timestamp: chrono::Utc::now().to_rfc3339(),
                        };
                        broadcast_event(&changed, connections).await;
                    }

                    emit_telemetry(
                        "agent-task-created",
                        &agent_id,
                        serde_json::json!({"taskId": task_id, "objective": objective}),
                        connections,
                    )
                    .await;
                }
                Some((prev_status, prev_progress)) => {
                    let status_changed = prev_status != status;
                    let progress_changed = (progress - prev_progress).abs() > 0.01;

                    if status_changed || progress_changed {
                        known.insert(composite_key.clone(), (status.to_string(), progress));

                        {
                            let mut store = task_store.write().await;
                            if let Some(tracked) = store.get_mut(&composite_key) {
                                tracked.status = ui_status.clone();
                                tracked.progress = Some(progress as f32);
                                if ui_status == "completed" || ui_status == "failed" {
                                    tracked.completed_at = Some(chrono::Utc::now().to_rfc3339());
                                }
                            }
                        }

                        let metrics = quality_score.map(|q| crate::types::TaskMetrics {
                            tokens_generated: 0,
                            tokens_per_sec: 0.0,
                            ttft_ms: 0,
                            inference_duration_ms: 0,
                            energy_wh: 0.0,
                            quality_score: Some(q),
                        });

                        let changed = AgUiEvent::TaskStatusChanged {
                            task_id: composite_key.clone(),
                            status: ui_status.clone(),
                            progress: Some(progress as f32),
                            result: None,
                            metrics,
                            timestamp: chrono::Utc::now().to_rfc3339(),
                        };
                        broadcast_event(&changed, connections).await;

                        if status_changed {
                            emit_telemetry(
                                &format!("agent-task-{ui_status}"),
                                &agent_id,
                                serde_json::json!({"taskId": task_id}),
                                connections,
                            )
                            .await;
                        }
                    }
                }
            }

            // Emit telemetry for tool calls
            if let Some(tool_calls) = task_val.get("tool_calls").and_then(|v| v.as_array()) {
                for tc in tool_calls {
                    let name = tc.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                    let success = tc.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
                    emit_telemetry(
                        "agent-tool-call",
                        &agent_id,
                        serde_json::json!({"tool": name, "success": success, "taskId": task_id}),
                        connections,
                    )
                    .await;
                }
            }
        }
    }
    Ok(())
}

fn hrm_status_to_ui(hrm_status: &str) -> String {
    match hrm_status {
        "Completed" => "completed",
        "Failed" => "failed",
        "Running" => "working",
        "Cancelled" => "failed",
        "Pending" | "Scheduled" => "submitted",
        _ => "submitted",
    }
    .to_string()
}

async fn emit_telemetry(
    event_type: &str,
    agent_id: &str,
    details: serde_json::Value,
    connections: &Arc<RwLock<HashMap<String, super::gateway::ConnectionInfo>>>,
) {
    let event = AgUiEvent::TelemetryEvent {
        event_type: event_type.to_string(),
        agent_id: agent_id.to_string(),
        details,
        timestamp: chrono::Utc::now().to_rfc3339(),
    };
    broadcast_event(&event, connections).await;
}
