use crate::{CodeUpdate, LiveReloadManager};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use warp::ws::{Message, WebSocket};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum WsMessage {
    SubmitPrompt {
        text: String,
    },
    Plan {
        parts: Vec<PlanPart>,
    },
    PartStream {
        part_id: String,
        chunk_type: String,
        content: String,
        done: bool,
    },
    ApplyPart {
        part_id: String,
    },
    AppliedPart {
        part_id: String,
        version_id: String,
    },
    CancelGeneration,
    Undo,
    Redo,
    UndoAvailable {
        can_undo: bool,
        can_redo: bool,
    },
    UserEdit {
        selector: String,
        action: String,
        before: String,
        after: String,
    },
    SaveSession {
        name: String,
    },
    Error {
        message: String,
    },
    CodeUpdate {
        update: CodeUpdate,
    },
    HistoryRequest {
        limit: usize,
    },
    HistoryResponse {
        versions: Vec<String>,
    },
    RollbackRequest {
        version_id: String,
    },
    RollbackResponse {
        success: bool,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanPart {
    pub id: String,
    pub name: String,
    pub description: String,
}

pub async fn handle_live_reload_ws(ws: WebSocket, manager: Arc<LiveReloadManager>) {
    let (mut ws_tx, mut ws_rx) = ws.split();

    let mut update_rx = manager.subscribe().await;

    let manager_for_recv = manager.clone();

    let receive_task = tokio::spawn(async move {
        while let Some(result) = ws_rx.next().await {
            if let Ok(msg) = result
                && let Ok(text) = msg.to_str()
                && let Ok(ws_msg) = serde_json::from_str::<WsMessage>(text)
            {
                match ws_msg {
                    WsMessage::HistoryRequest { limit } => {
                        let versions = manager_for_recv.get_version_history(limit).await;
                        let version_ids: Vec<String> =
                            versions.iter().map(|v| v.version_id.clone()).collect();

                        let response = WsMessage::HistoryResponse {
                            versions: version_ids,
                        };

                        if let Ok(json) = serde_json::to_string(&response) {
                            tracing::debug!("History response: {json}");
                        }
                    }
                    WsMessage::RollbackRequest { version_id } => {
                        let result = manager_for_recv.rollback(&version_id).await;
                        let response = match result {
                            Ok(()) => WsMessage::RollbackResponse {
                                success: true,
                                message: "Rollback successful".to_string(),
                            },
                            Err(e) => WsMessage::RollbackResponse {
                                success: false,
                                message: e.to_string(),
                            },
                        };

                        if let Ok(json) = serde_json::to_string(&response) {
                            tracing::debug!("Rollback response: {json}");
                        }
                    }
                    _ => {}
                }
            }
        }
    });

    let send_task = tokio::spawn(async move {
        while let Some(update) = update_rx.recv().await {
            let ws_msg = WsMessage::CodeUpdate { update };

            if let Ok(json) = serde_json::to_string(&ws_msg)
                && ws_tx.send(Message::text(json)).await.is_err()
            {
                break;
            }
        }
    });

    tokio::select! {
        _ = receive_task => {},
        _ = send_task => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_serialization() {
        let update = CodeUpdate {
            id: "test-123".to_string(),
            html: Some("<div>Test</div>".to_string()),
            css: None,
            javascript: None,
            timestamp: chrono::Utc::now(),
            source: crate::UpdateSource::LlmGenerated,
        };

        let msg = WsMessage::CodeUpdate { update };
        let json = serde_json::to_string(&msg).unwrap();
        // With #[serde(tag = "type", content = "data")], the JSON will be:
        // {"type":"CodeUpdate","data":{...}}
        assert!(json.contains("CodeUpdate"));
        assert!(json.contains("\"type\""));
    }

    #[test]
    fn test_history_request() {
        let msg = WsMessage::HistoryRequest { limit: 10 };
        let json = serde_json::to_string(&msg).unwrap();
        // With #[serde(tag = "type", content = "data")], the JSON will be:
        // {"type":"HistoryRequest","data":10}
        assert!(json.contains("HistoryRequest"));
        assert!(json.contains("10"));
    }
}
