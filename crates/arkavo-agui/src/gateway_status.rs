use crate::agent_connection::TelemetryEvent;
use crate::debug_handler::DebugHandler;
use crate::types::*;
use axum::extract::ws::{Message, WebSocket};
use axum::{extract::State, response::Response};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

pub async fn telemetry_websocket_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(state): State<super::gateway::AppState>,
) -> Response {
    ws.on_upgrade(|socket| handle_telemetry_websocket(socket, state.telemetry_rx))
}

pub async fn debug_websocket_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(state): State<super::gateway::AppState>,
) -> Response {
    ws.on_upgrade(|socket| handle_debug_websocket(socket, state.debug_handler))
}

async fn handle_telemetry_websocket(
    mut ws: WebSocket,
    telemetry_rx: Arc<RwLock<mpsc::Receiver<TelemetryEvent>>>,
) {
    println!("New telemetry WebSocket connection");
    let mut rx = telemetry_rx.write().await;

    while let Some(event) = rx.recv().await {
        if let Ok(json) = serde_json::to_string(&event)
            && ws.send(Message::Text(json)).await.is_err()
        {
            break;
        }
    }

    println!("Telemetry WebSocket connection closed");
}

async fn handle_debug_websocket(mut ws: WebSocket, debug_handler: Option<Arc<DebugHandler>>) {
    if let Some(handler) = debug_handler {
        println!("New debug WebSocket connection");
        let (tx, mut rx) = mpsc::channel::<AgUiEvent>(100);

        let ws_clone = Arc::new(tokio::sync::Mutex::new(ws));
        let ws_for_forward = ws_clone.clone();
        let forward_task = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Ok(json) = serde_json::to_string(&event) {
                    let mut ws_guard = ws_for_forward.lock().await;
                    if ws_guard.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                }
            }
        });

        let ws_for_recv = ws_clone.clone();
        loop {
            let msg = {
                let mut ws_guard = ws_for_recv.lock().await;
                ws_guard.recv().await
            };

            match msg {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(cmd) = serde_json::from_str::<DebugCommand>(&text) {
                        dispatch_debug_command(cmd, &handler, &tx).await;
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Err(_)) => break,
                _ => {}
            }
        }

        forward_task.abort();
        println!("Debug WebSocket connection closed");
    } else {
        let error = AgUiEvent::Error {
            code: "DEBUG_DISABLED".to_string(),
            message: "Debug handler not configured".to_string(),
        };
        if let Ok(json) = serde_json::to_string(&error) {
            let _ = ws.send(Message::Text(json)).await;
        }
    }
}

async fn dispatch_debug_command(
    cmd: DebugCommand,
    handler: &DebugHandler,
    tx: &mpsc::Sender<AgUiEvent>,
) {
    match cmd {
        DebugCommand::SubscribeSession { session_id } => {
            let _ = handler.subscribe_to_session(session_id, tx.clone()).await;
        }
        DebugCommand::UnsubscribeSession { session_id } => {
            handler.unsubscribe_session(&session_id).await;
            let event = AgUiEvent::StateDelta {
                patch: vec![JsonPatch::Add {
                    path: "/unsubscribed".to_string(),
                    value: serde_json::Value::String(session_id.clone()),
                }],
                event_id: format!("unsubscribed-{session_id}"),
            };
            let _ = tx.send(event).await;
        }
        DebugCommand::GetRecentSessions { limit } => {
            if let Ok(sessions) = handler.get_recent_sessions(limit).await {
                let event = AgUiEvent::StateDelta {
                    patch: vec![JsonPatch::Add {
                        path: "/sessions".to_string(),
                        value: serde_json::to_value(sessions).unwrap_or(serde_json::Value::Null),
                    }],
                    event_id: format!("sessions-{}", uuid::Uuid::new_v4()),
                };
                let _ = tx.send(event).await;
            }
        }
        DebugCommand::GetActiveSessions => {
            let sessions = handler.get_active_sessions().await;
            let event = AgUiEvent::StateDelta {
                patch: vec![JsonPatch::Add {
                    path: "/active_sessions".to_string(),
                    value: serde_json::to_value(sessions).unwrap_or(serde_json::Value::Null),
                }],
                event_id: format!("active-sessions-{}", uuid::Uuid::new_v4()),
            };
            let _ = tx.send(event).await;
        }
        DebugCommand::AttachToAgent { agent_id } => {
            let session_id = uuid::Uuid::new_v4().to_string();
            handler.attach_to_agent(agent_id.clone(), session_id).await;
            let event = AgUiEvent::StateDelta {
                patch: vec![JsonPatch::Add {
                    path: "/attached_agent".to_string(),
                    value: serde_json::Value::String(agent_id),
                }],
                event_id: format!("attached-{}", uuid::Uuid::new_v4()),
            };
            let _ = tx.send(event).await;
        }
        DebugCommand::DetachFromAgent { agent_id } => {
            handler.detach_from_agent(&agent_id).await;
            let event = AgUiEvent::StateDelta {
                patch: vec![JsonPatch::Remove {
                    path: format!("/attached_agent/{agent_id}"),
                }],
                event_id: format!("detached-{}", uuid::Uuid::new_v4()),
            };
            let _ = tx.send(event).await;
        }
        DebugCommand::StartRecording { session_id } => {
            handler.start_recording(session_id.clone()).await;
            let event = AgUiEvent::StateDelta {
                patch: vec![JsonPatch::Add {
                    path: "/recording".to_string(),
                    value: serde_json::json!({ "session_id": session_id, "status": "started" }),
                }],
                event_id: format!("recording-started-{}", uuid::Uuid::new_v4()),
            };
            let _ = tx.send(event).await;
        }
        DebugCommand::StopRecording { session_id } => {
            handler.stop_recording(&session_id).await;
            let event = AgUiEvent::StateDelta {
                patch: vec![JsonPatch::Add {
                    path: "/recording".to_string(),
                    value: serde_json::json!({ "session_id": session_id, "status": "stopped" }),
                }],
                event_id: format!("recording-stopped-{}", uuid::Uuid::new_v4()),
            };
            let _ = tx.send(event).await;
        }
        DebugCommand::GetSessionEvents { session_id, limit } => {
            match handler.get_session_events(&session_id, limit).await {
                Ok(events) => {
                    let event = AgUiEvent::StateDelta {
                        patch: vec![JsonPatch::Add {
                            path: "/session_events".to_string(),
                            value: serde_json::json!({
                                "session_id": session_id,
                                "events": events,
                                "count": events.len()
                            }),
                        }],
                        event_id: format!("events-{}", uuid::Uuid::new_v4()),
                    };
                    let _ = tx.send(event).await;
                }
                Err(e) => {
                    let error = AgUiEvent::Error {
                        code: "EVENT_FETCH_FAILED".to_string(),
                        message: format!("Failed to fetch events: {e}"),
                    };
                    let _ = tx.send(error).await;
                }
            }
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum DebugCommand {
    SubscribeSession {
        session_id: String,
    },
    UnsubscribeSession {
        session_id: String,
    },
    GetRecentSessions {
        limit: usize,
    },
    GetActiveSessions,
    AttachToAgent {
        agent_id: String,
    },
    DetachFromAgent {
        agent_id: String,
    },
    StartRecording {
        session_id: String,
    },
    StopRecording {
        session_id: String,
    },
    GetSessionEvents {
        session_id: String,
        limit: Option<usize>,
    },
}
