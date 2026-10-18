use crate::agent_connection::AgentConnection;
use crate::gateway_events::short_id;
use crate::gateway_routing::{broadcast_event, build_routing_candidates, update_task_status};
use crate::types::*;
use arkavo_protocol::task_watch::Watch;
use arkavo_protocol::types::TaskGetResponse;
use arkavo_router::learning::LearningModule;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

#[allow(clippy::too_many_arguments)]
pub async fn handle_submit_task(
    description: String,
    request_id: Option<String>,
    target_agent: Option<String>,
    agents: &Arc<RwLock<Vec<serde_json::Value>>>,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    task_store: &Arc<RwLock<HashMap<String, super::gateway::TrackedTask>>>,
    connections: &Arc<RwLock<HashMap<String, super::gateway::ConnectionInfo>>>,
    learning_module: &Arc<RwLock<LearningModule>>,
    routing_history: &Arc<RwLock<VecDeque<RoutingRecord>>>,
    lesson_tx: &Option<mpsc::Sender<arkavo_router::learning::Lesson>>,
    lesson_store: &Arc<RwLock<Vec<arkavo_router::learning::Lesson>>>,
    asset_fetcher: Option<Arc<dyn crate::gateway_assets::AssetFetcher>>,
    gateway_notices: crate::gateway_notices::GatewayNotices,
) -> Result<(), Box<dyn std::error::Error>> {
    let task_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    // Classify task by keywords for per-category routing
    let task_category = arkavo_router::classify_task_keywords(&description);
    let category_str = task_category.as_str().to_string();

    // A browser that lost its receipt (a dropped socket, a slow gateway)
    // sends the same request again. It gets the first task's receipt back
    // rather than a second task doing the same work. The check and the
    // insert share one lock: a resend from a reconnected view runs
    // alongside the first submission, which may still be routing.
    let tracked = super::gateway::TrackedTask {
        id: task_id.clone(),
        description: description.clone(),
        target_agent: None,
        status: "submitted".to_string(),
        progress: Some(0.0),
        result: None,
        created_at: now.clone(),
        completed_at: None,
        task_category: Some(category_str.clone()),
        first_working_at: None,
        source: "ui".to_string(),
        summary: None,
        request_id,
        assets: Vec::new(),
        asset_note: None,
    };
    let existing = {
        let mut store = task_store.write().await;
        let existing = tracked.request_id.as_deref().and_then(|request| {
            store
                .values()
                .find(|task| task.request_id.as_deref() == Some(request))
                .cloned()
        });
        if existing.is_none() {
            store.insert(task_id.clone(), tracked.clone());
            super::gateway::prune_tasks(&mut store);
        }
        existing
    };
    if let Some(task) = existing {
        broadcast_event(&receipt(&task), connections).await;
        return Ok(());
    }

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
    tracing::debug!(
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

    if let Some(task) = task_store.write().await.get_mut(&task_id) {
        task.target_agent = selected_agent.clone();
    }
    // Every open view hears the receipt, so one that reconnected after
    // sending still learns its task was accepted.
    broadcast_event(&receipt(&tracked), connections).await;

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
                    asset_fetcher.as_ref(),
                    &gateway_notices,
                )
                .await;
            });
        }
    }

    Ok(())
}

/// The receipt for an accepted submission.
fn receipt(task: &super::gateway::TrackedTask) -> AgUiEvent {
    AgUiEvent::TaskSubmitted {
        request_id: task.request_id.clone(),
        task_id: task.id.clone(),
        description: Some(task.description.clone()),
        summary: task.summary.clone(),
        status: task.status.clone(),
        source: Some(task.source.clone()),
        timestamp: task.created_at.clone(),
    }
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
    tracing::debug!(
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
    asset_fetcher: Option<&Arc<dyn crate::gateway_assets::AssetFetcher>>,
    gateway_notices: &crate::gateway_notices::GatewayNotices,
) {
    // Submit task to agent — returns immediately with task_id
    let agent_task_id = match conn.send_request("message/send", request, task_id).await {
        Ok(response) => {
            let tid = response
                .get("task_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            tracing::debug!(
                "AG-UI: Task {} submitted to {}, agent_task_id={}",
                short_id(task_id),
                agent_id,
                &tid[..tid.len().min(8)]
            );
            tid
        }
        Err(e) => {
            tracing::warn!(
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
            crate::gateway_notices::task_failed(
                gateway_notices,
                connections,
                task_id,
                agent_id,
                &format!("The agent did not take the task: {e}"),
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

    // Poll tasks/get until terminal status, for as long as the task reports
    // change (CLI-019): a delegated image job can take far longer than
    // five minutes and still be moving.
    let mut progress = 0.15_f32;
    let poll_interval = std::time::Duration::from_secs(2);
    let mut watch = Watch::default();
    let quiet = loop {
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
                        let parts = answer_parts(&poll_response);
                        let result_text = answer_text(&parts);
                        // A model that wrote its tool call out as text did
                        // nothing: the operator gets a failure with a retry,
                        // never control tokens, and routing learns it failed.
                        if let Some(marker) = unparsed_tool_call(&parts) {
                            let why = format!(
                                "The agent's model wrote a tool call ({marker}) instead of making it, so nothing was done."
                            );
                            tracing::warn!(task = %short_id(task_id), agent_id, "{why}");
                            update_task_status(
                                task_store,
                                task_id,
                                "failed",
                                None,
                                Some(why.clone()),
                                connections,
                                learning_module,
                                routing_history,
                                lesson_tx,
                            )
                            .await;
                            crate::gateway_notices::task_failed(
                                gateway_notices,
                                connections,
                                task_id,
                                agent_id,
                                &why,
                            )
                            .await;
                            return;
                        }

                        tracing::debug!(
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
                        crate::gateway_assets::deliver(
                            asset_fetcher,
                            task_id,
                            &parts,
                            task_store,
                            connections,
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

                        tracing::warn!(
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
                            Some(error_msg.clone()),
                            connections,
                            learning_module,
                            routing_history,
                            lesson_tx,
                        )
                        .await;
                        crate::gateway_notices::task_failed(
                            gateway_notices,
                            connections,
                            task_id,
                            agent_id,
                            &error_msg,
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
                        if let Err(quiet) =
                            keep_following(&mut watch, &agent_task_id, &poll_response)
                        {
                            break quiet;
                        }
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
                            assets: Vec::new(),
                            asset_note: None,
                            timestamp: chrono::Utc::now().to_rfc3339(),
                        };
                        broadcast_event(&event, connections).await;
                    }
                }
            }
            Err(e) => {
                // A poll the agent turned away, its rate limiter included,
                // is no change: the task goes on.
                tracing::warn!("AG-UI: Task {} poll error: {}", short_id(task_id), e);
                if let Err(quiet) = watch.unanswered(&agent_task_id) {
                    break quiet;
                }
            }
        }
    };

    tracing::warn!("AG-UI: Task {}: {quiet}", short_id(task_id));
    update_task_status(
        task_store,
        task_id,
        "failed",
        None,
        Some(quiet.clone()),
        connections,
        learning_module,
        routing_history,
        lesson_tx,
    )
    .await;
    crate::gateway_notices::task_failed(gateway_notices, connections, task_id, agent_id, &quiet)
        .await;
}

/// The control tokens a model writes when it spells out a tool call instead
/// of making it, as Gemma, Qwen, Mistral and Llama do.
/// The marker of a tool call the model wrote out as its whole answer. Only
/// an answer that opens with one and came with nothing else counts: an
/// answer that explains the syntax, or one that delivered files beside its
/// text, did its work.
fn unparsed_tool_call(parts: &[arkavo_protocol::types::MessagePart]) -> Option<&'static str> {
    let mut text = String::new();
    for part in parts {
        match part {
            arkavo_protocol::types::MessagePart::Text { content } => text.push_str(content),
            _ => return None,
        }
    }
    let text = text.trim_start();
    [
        "<|tool_call>",
        "<tool_call>",
        "[TOOL_CALLS]",
        "<|python_tag|>",
    ]
    .into_iter()
    .find(|marker| text.starts_with(marker))
}

/// The parts of a completed task's answer, as the agent sent them.
fn answer_parts(report: &serde_json::Value) -> Vec<arkavo_protocol::types::MessagePart> {
    report
        .get("result")
        .and_then(|result| result.get("parts"))
        .and_then(|parts| serde_json::from_value(parts.clone()).ok())
        .unwrap_or_default()
}

/// The text of an answer, one line per text part, as the CLI prints it: the
/// first part alone dropped the rest of a multi-part answer.
fn answer_text(parts: &[arkavo_protocol::types::MessagePart]) -> String {
    let text = parts
        .iter()
        .filter_map(|part| match part {
            arkavo_protocol::types::MessagePart::Text { content } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        "Task completed".to_string()
    } else {
        text
    }
}

/// Note a poll that found the task still going: `Err` once it has
/// reported nothing new for the limit. A report this gateway cannot read
/// counts as no change.
fn keep_following(
    watch: &mut Watch,
    agent_task_id: &str,
    report: &serde_json::Value,
) -> Result<(), String> {
    match serde_json::from_value::<TaskGetResponse>(report.clone()) {
        Ok(task) => watch.observe(&task).map(drop),
        Err(_) => watch.unanswered(agent_task_id),
    }
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

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use std::time::Duration;

    use arkavo_test_macros::spec;
    use serde_json::json;

    use super::*;

    async fn submit(state: &crate::gateway::AppState, request_id: &str) {
        handle_submit_task(
            "Make a gold coin sprite".into(),
            Some(request_id.into()),
            None,
            &state.agents,
            &state.agent_connections,
            &state.task_store,
            &state.connections,
            &state.learning_module,
            &state.routing_history,
            &state.lesson_tx,
            &state.lesson_store,
            None,
            state.gateway_notices.clone(),
        )
        .await
        .expect("submitted");
    }

    /// Regression (AGUI-023): a browser that lost its receipt and sent the
    /// draft again opened a second task, and the receipt went only to the
    /// socket that sent it, so a view that reconnected never heard it. The
    /// same request is one task, and every view hears its receipt.
    #[spec("AGUI-023")]
    #[tokio::test]
    async fn a_resent_request_is_one_task_with_its_receipt_to_every_view() {
        let state = crate::test_support::app_state();
        let (tx, mut rx) = mpsc::channel(8);
        state.connections.write().await.insert(
            "reconnected-view".into(),
            crate::gateway::ConnectionInfo {
                _ws_tx: tx,
                _agent_id: None,
                subscriptions: Vec::new(),
                current_plan: None,
                current_prompt: None,
            },
        );

        submit(&state, "draft-1").await;
        submit(&state, "draft-1").await;

        assert_eq!(state.task_store.read().await.len(), 1);
        let first = serde_json::to_value(rx.recv().await.unwrap()).unwrap();
        let second = serde_json::to_value(rx.recv().await.unwrap()).unwrap();
        assert_eq!(first["type"], "taskSubmitted");
        assert_eq!(first["requestId"], "draft-1");
        assert_eq!(
            second["taskId"], first["taskId"],
            "the resend gets the same task"
        );
    }

    /// Regression (AGUI-023): the resend was checked before the first
    /// submission was stored, so a resend from a reconnected view, sent
    /// while the first was still telling a stalled view about it, opened a
    /// second task.
    #[spec("AGUI-023")]
    #[tokio::test]
    async fn a_resend_while_the_first_is_still_being_announced_is_one_task() {
        let state = crate::test_support::app_state();
        let (tx, mut stalled) = mpsc::channel(1);
        let filler = AgUiEvent::OperatorNoticeResolved {
            agent_id: "artist".into(),
            notice_id: "n1".into(),
        };
        tx.send(filler).await.unwrap();
        state.connections.write().await.insert(
            "stalled-view".into(),
            crate::gateway::ConnectionInfo {
                _ws_tx: tx,
                _agent_id: None,
                subscriptions: Vec::new(),
                current_plan: None,
                current_prompt: None,
            },
        );
        let drain = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            while tokio::time::timeout(Duration::from_millis(50), stalled.recv())
                .await
                .is_ok_and(|event| event.is_some())
            {}
        };

        tokio::join!(submit(&state, "draft-2"), submit(&state, "draft-2"), drain);

        assert_eq!(state.task_store.read().await.len(), 1);
    }

    fn working(step: u32) -> serde_json::Value {
        json!({
            "task_id": "t1",
            "status": "working",
            "progress": {"message": format!("image 1 of 1: step {step} of 8")}
        })
    }

    /// Regression (AGUI-022): the gateway stopped polling five minutes
    /// after a task was submitted and marked it failed, while a delegated
    /// image job was still reporting steps. It follows a task for as long
    /// as the task reports change.
    #[spec("AGUI-022")]
    #[test]
    fn a_task_is_followed_while_it_reports_change() {
        let mut watch = Watch::with_limit(Duration::from_millis(50));
        for step in 1..=4 {
            std::thread::sleep(Duration::from_millis(20));
            assert_eq!(keep_following(&mut watch, "t1", &working(step)), Ok(()));
        }

        std::thread::sleep(Duration::from_millis(60));
        let quiet = keep_following(&mut watch, "t1", &working(4)).unwrap_err();
        assert!(quiet.contains("Task t1 is still running"), "{quiet}");
        assert!(quiet.contains("tasks/get"), "{quiet}");
    }

    /// Regression (AGUI-028): the MacBook's model answered a coin request
    /// with "<|tool_call>call:generate_image prompt: ..." and the workspace
    /// showed it as the completed output.
    #[spec("AGUI-028")]
    #[test]
    fn a_tool_call_written_out_is_not_an_answer() {
        use arkavo_protocol::types::MessagePart;
        let text = |content: &str| MessagePart::Text {
            content: content.into(),
        };
        let leaked =
            "<|tool_call>call:generate_image\nprompt: A transparent sprite of a gold coin.";
        assert_eq!(unparsed_tool_call(&[text(leaked)]), Some("<|tool_call>"));
        assert_eq!(
            unparsed_tool_call(&[text("\n[TOOL_CALLS][{\"name\": \"x\"}]")]),
            Some("[TOOL_CALLS]")
        );
        assert_eq!(
            unparsed_tool_call(&[text("Your gold coin sprite is attached.")]),
            None
        );
    }

    /// AGUI-028: an answer that only mentions a marker, or that delivered
    /// files beside its text, is an answer.
    #[spec("AGUI-028")]
    #[test]
    fn an_answer_that_mentions_tool_call_syntax_is_an_answer() {
        use arkavo_protocol::types::MessagePart;
        let text = |content: &str| MessagePart::Text {
            content: content.into(),
        };
        assert_eq!(
            unparsed_tool_call(&[text(
                "Mistral marks a call with [TOOL_CALLS] followed by a JSON list."
            )]),
            None
        );
        let file = MessagePart::File {
            name: "coin.png".into(),
            mime_type: "image/png".into(),
            data: "iroh://coin".into(),
            is_uri: true,
        };
        assert_eq!(
            unparsed_tool_call(&[text("<|tool_call>call:generate_image"), file]),
            None
        );
    }

    /// AGUI-022: a report the gateway cannot read, like a poll the agent
    /// turned away, is no change.
    #[spec("AGUI-022")]
    #[test]
    fn an_unreadable_report_is_no_change() {
        let mut watch = Watch::with_limit(Duration::from_millis(50));
        assert_eq!(keep_following(&mut watch, "t1", &json!({})), Ok(()));
        std::thread::sleep(Duration::from_millis(60));
        assert!(keep_following(&mut watch, "t1", &json!({})).is_err());
    }
}
