use crate::agent_connection::AgentConnection;
use crate::budget_handler::BudgetHandler;
use crate::{gateway_config, gateway_events};
use crate::types::*;
use axum::extract::ws::{Message, WebSocket};
use axum::{extract::State, response::Response};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

pub async fn websocket_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(state): State<super::gateway::AppState>,
) -> Response {
    ws.on_upgrade(|socket| {
        handle_websocket(
            socket,
            state.connections,
            state.agents,
            state.agent_connections,
            state.budget_handler,
            state.initial_prompt,
            state.cost_handler,
        )
    })
}

async fn handle_websocket(
    ws: WebSocket,
    connections: Arc<RwLock<HashMap<String, super::gateway::ConnectionInfo>>>,
    agents: Arc<RwLock<Vec<serde_json::Value>>>,
    agent_connections: Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    budget_handler: Arc<RwLock<BudgetHandler>>,
    initial_prompt: Arc<RwLock<Option<String>>>,
    cost_handler: Arc<RwLock<crate::cost_handler::CostHandler>>,
) {
    use futures::sink::SinkExt;
    use futures::stream::StreamExt;

    let session_id = uuid::Uuid::new_v4().to_string();
    let (tx, mut rx) = mpsc::channel::<AgUiEvent>(32);
    let (mut ws_write, mut ws_read) = ws.split();

    let forward_task = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if let Ok(json) = serde_json::to_string(&event)
                && ws_write.send(Message::Text(json)).await.is_err()
            {
                break;
            }
        }
    });

    println!("AG-UI: New WebSocket connection: {session_id}");

    {
        let conn_info = super::gateway::ConnectionInfo {
            _ws_tx: tx.clone(),
            _agent_id: None,
            subscriptions: Vec::new(),
            current_plan: None,
            current_prompt: None,
        };
        connections
            .write()
            .await
            .insert(session_id.clone(), conn_info);
    }

    let prompt_guard = initial_prompt.read().await;
    if let Some(prompt_text) = prompt_guard.as_ref() {
        println!("AG-UI: Auto-submitting initial prompt: {prompt_text}");
        let submit_event = AgUiEvent::SubmitPrompt {
            text: prompt_text.clone(),
        };
        drop(prompt_guard);
        if let Err(e) = dispatch_event(
            submit_event, &session_id, &connections, &agents,
            &agent_connections, &budget_handler, &cost_handler, &tx,
        ).await {
            eprintln!("AG-UI: Error auto-submitting initial prompt: {e}");
        }
    }

    loop {
        let msg = ws_read.next().await;
        match msg {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<AgUiEvent>(&text) {
                Ok(event) => {
                    if let Err(e) = dispatch_event(
                        event, &session_id, &connections, &agents,
                        &agent_connections, &budget_handler, &cost_handler, &tx,
                    ).await {
                        eprintln!("AG-UI: Error handling event: {e}");
                    }
                }
                Err(e) => {
                    eprintln!("AG-UI: Failed to parse event: {e}");
                    let _ = tx.send(AgUiEvent::Error {
                        code: "INVALID_EVENT".to_string(),
                        message: format!("Failed to parse event: {e}"),
                    }).await;
                }
            },
            Some(Ok(Message::Close(_))) | None => break,
            Some(Err(e)) => { eprintln!("AG-UI: WebSocket error: {e}"); break; }
            _ => {}
        }
    }

    if let Some(mut conn_info) = connections.write().await.remove(&session_id) {
        for mut sub in conn_info.subscriptions.drain(..) {
            sub.cancel();
        }
    }
    forward_task.abort();
    println!("AG-UI: WebSocket connection closed: {session_id}");
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_event(
    event: AgUiEvent,
    session_id: &str,
    connections: &Arc<RwLock<HashMap<String, super::gateway::ConnectionInfo>>>,
    agents: &Arc<RwLock<Vec<serde_json::Value>>>,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    budget_handler: &Arc<RwLock<BudgetHandler>>,
    cost_handler: &Arc<RwLock<crate::cost_handler::CostHandler>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    match event {
        AgUiEvent::Connect { agent_id, agui_version, since_event_id: _ } => {
            handle_connect(agent_id, agui_version, session_id, connections, agents, tx).await?;
        }
        AgUiEvent::ChatOpen { agent_id } => {
            handle_chat_open(agent_id, session_id, connections, agent_connections, tx).await?;
        }
        AgUiEvent::ChatClose { agent_id } => {
            let ac = agent_connections.read().await;
            if let Some(c) = ac.get(&agent_id)
                && let Err(e) = c.unsubscribe_chat(&agent_id).await
            {
                eprintln!("Failed to unsubscribe from chat: {e}");
            }
        }
        AgUiEvent::UserMessage { agent_id, content, attachments: _ } => {
            handle_user_message(agent_id, content, agent_connections, tx).await?;
        }
        AgUiEvent::GetBudgetStatus { .. }
        | AgUiEvent::SetAgentBudget { .. }
        | AgUiEvent::ResetBudgetWindow { .. } => {
            budget_handler.read().await.handle_event(&event, tx).await?;
        }
        AgUiEvent::GetCostMetrics { .. }
        | AgUiEvent::GetROIDashboard
        | AgUiEvent::GetCostPrediction { .. } => {
            cost_handler.read().await.handle_event(&event, tx).await?;
        }
        AgUiEvent::GetAgentConfig { agent_id, include_backups } => {
            gateway_config::handle_get_config(agent_id, include_backups, agent_connections, tx).await?;
        }
        AgUiEvent::UpdateAgentConfig { agent_id, content, expected_version, create_backup } => {
            gateway_config::handle_update_config(agent_id, content, expected_version, create_backup, agent_connections, tx).await?;
        }
        AgUiEvent::ValidateAgentConfig { agent_id, content } => {
            gateway_config::handle_validate_config(agent_id, content, agent_connections, tx).await?;
        }
        AgUiEvent::RestoreAgentConfig { agent_id, backup_filename } => {
            gateway_config::handle_restore_config(agent_id, backup_filename, agent_connections, tx).await?;
        }
        AgUiEvent::SubmitPrompt { text } => {
            gateway_events::handle_submit_prompt(text, session_id, connections, tx).await?;
        }
        AgUiEvent::RequestStatus => {
            gateway_events::handle_request_status(connections, tx).await?;
        }
        AgUiEvent::RequestMeshStatus => {
            gateway_events::handle_request_mesh_status(agents, tx).await?;
        }
        AgUiEvent::ApplyPart { part_id } => {
            gateway_events::handle_apply_part(part_id, session_id, connections, tx).await?;
        }
        AgUiEvent::RequestTaskList => {
            gateway_events::handle_request_task_list(session_id, connections, tx).await?;
        }
        AgUiEvent::SubmitTask { description, target_agent: _ } => {
            gateway_events::handle_submit_task(description, agents, connections, tx).await?;
        }
        _ => { println!("AG-UI: Received event: {event:?}"); }
    }
    Ok(())
}

async fn handle_connect(
    agent_id: String, agui_version: String, session_id: &str,
    connections: &Arc<RwLock<HashMap<String, super::gateway::ConnectionInfo>>>,
    agents: &Arc<RwLock<Vec<serde_json::Value>>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("AG-UI: Connect request for agent {agent_id} (version {agui_version})");
    let agents_list = agents.read().await;
    let found = agents_list
        .iter()
        .any(|a| a.get("id").and_then(|v| v.as_str()) == Some(&agent_id));
    if found {
        let ci = super::gateway::ConnectionInfo {
            _ws_tx: tx.clone(), _agent_id: Some(agent_id.clone()),
            subscriptions: Vec::new(), current_plan: None, current_prompt: None,
        };
        drop(agents_list);
        connections.write().await.insert(session_id.to_string(), ci);
        tx.send(AgUiEvent::StateSnapshot {
            state: serde_json::json!({}), event_id: uuid::Uuid::new_v4().to_string(),
        }).await?;
        tx.send(AgUiEvent::MessagesSnapshot {
            messages: vec![], event_id: uuid::Uuid::new_v4().to_string(),
        }).await?;
        tx.send(AgUiEvent::LifecycleStart { session_id: session_id.to_string() }).await?;
    } else {
        tx.send(AgUiEvent::Error {
            code: "AGENT_NOT_FOUND".to_string(), message: format!("Agent {agent_id} not found"),
        }).await?;
    }
    Ok(())
}

async fn handle_chat_open(
    agent_id: String, session_id: &str,
    connections: &Arc<RwLock<HashMap<String, super::gateway::ConnectionInfo>>>,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let ac = agent_connections.read().await;
    if let Some(conn) = ac.get(&agent_id) {
        match conn.subscribe_to_chat(agent_id.clone(), tx.clone(), None).await {
            Ok(sub) => {
                let mut cg = connections.write().await;
                if let Some(ci) = cg.get_mut(session_id) { ci.subscriptions.push(sub); }
            }
            Err(e) => {
                tx.send(AgUiEvent::Error {
                    code: "SUBSCRIPTION_FAILED".to_string(),
                    message: format!("Failed to subscribe to chat: {e}"),
                }).await?;
            }
        }
    } else {
        tx.send(AgUiEvent::Error {
            code: "AGENT_NOT_CONNECTED".to_string(),
            message: format!("Agent {agent_id} is not connected"),
        }).await?;
    }
    Ok(())
}

async fn handle_user_message(
    agent_id: String, content: String,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let ac = agent_connections.read().await;
    if let Some(conn) = ac.get(&agent_id) {
        if let Err(e) = conn.send_user_message(&agent_id, content).await {
            tx.send(AgUiEvent::Error {
                code: "MESSAGE_SEND_FAILED".to_string(),
                message: format!("Failed to send message: {e}"),
            }).await?;
        }
    } else {
        tx.send(AgUiEvent::Error {
            code: "AGENT_NOT_CONNECTED".to_string(),
            message: format!("Agent {agent_id} is not connected"),
        }).await?;
    }
    Ok(())
}
