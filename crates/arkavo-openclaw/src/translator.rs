use arkavo_protocol::transport::{A2aRequest, A2aResponse, JsonRpcError};
use serde_json::Value;
use uuid::Uuid;

use crate::protocol::{EventFrame, OpenClawError, RequestFrame, ResponseFrame};

/// Translate an inbound OpenClaw request frame into an A2A JSON-RPC request.
///
/// Maps OpenClaw method names to their A2A equivalents and restructures
/// parameters to match the A2A schema.
pub fn openclaw_req_to_a2a(frame: &RequestFrame) -> Result<A2aRequest, TranslatorError> {
    let (method, params) = match frame.method.as_str() {
        "chat" | "send_message" => {
            let content = extract_content(frame.params.as_ref())?;
            let params = serde_json::json!({
                "request": {
                    "message": {
                        "parts": [{"type": "text", "content": content}]
                    }
                }
            });
            ("message/send", params)
        }
        "task" | "create_task" => {
            let params = frame
                .params
                .clone()
                .unwrap_or_else(|| Value::Object(serde_json::Map::default()));
            let wrapped = serde_json::json!({
                "request": {
                    "message": {
                        "parts": [{"type": "text", "content": params.to_string()}]
                    },
                    "metadata": {"source": "openclaw", "original_method": frame.method}
                }
            });
            ("message/send", wrapped)
        }
        "discover" | "list_agents" => {
            let params = frame
                .params
                .clone()
                .unwrap_or_else(|| Value::Object(serde_json::Map::default()));
            ("agent_discover", params)
        }
        "status" => {
            let params = frame
                .params
                .clone()
                .unwrap_or_else(|| Value::Object(serde_json::Map::default()));
            ("tasks/get", params)
        }
        "cancel" => {
            let params = frame
                .params
                .clone()
                .unwrap_or_else(|| Value::Object(serde_json::Map::default()));
            ("tasks/cancel", params)
        }
        other => {
            let params = frame
                .params
                .clone()
                .unwrap_or_else(|| Value::Object(serde_json::Map::default()));
            (other, params)
        }
    };

    Ok(A2aRequest {
        jsonrpc: "2.0".to_string(),
        id: Uuid::new_v4(),
        method: method.to_string(),
        params,
    })
}

/// Translate an A2A JSON-RPC response into an OpenClaw response frame.
pub fn a2a_response_to_openclaw(openclaw_id: &str, response: &A2aResponse) -> ResponseFrame {
    match response {
        A2aResponse::Success { result, .. } => ResponseFrame {
            id: openclaw_id.to_string(),
            result: Some(result.clone()),
            error: None,
        },
        A2aResponse::Error { error, .. } => ResponseFrame {
            id: openclaw_id.to_string(),
            result: None,
            error: Some(OpenClawError {
                code: error.code,
                message: error.message.clone(),
                data: error.data.clone(),
            }),
        },
    }
}

/// Translate an A2A event (streaming delta, broadcast) into an OpenClaw event frame.
pub fn a2a_event_to_openclaw(method: &str, data: Value) -> EventFrame {
    let topic = match method {
        "message/stream" | "chat_stream" => "chat.delta",
        "agent_broadcast" => "status",
        other => other,
    };
    EventFrame {
        topic: topic.to_string(),
        data,
    }
}

/// Translate an A2A JSON-RPC request into an OpenClaw request frame (outbound direction).
///
/// Used by the client transport to convert A2A requests before sending to an OpenClaw gateway.
pub fn a2a_to_openclaw_req(request: &A2aRequest) -> RequestFrame {
    let method = match request.method.as_str() {
        "message/send" => "chat",
        "agent_discover" => "discover",
        "tasks/get" => "status",
        "tasks/cancel" => "cancel",
        other => other,
    };
    RequestFrame {
        id: request.id.to_string(),
        method: method.to_string(),
        params: Some(request.params.clone()),
    }
}

/// Translate an OpenClaw response frame back into an A2A JSON-RPC response.
///
/// Used by the client transport after receiving a response from an OpenClaw gateway.
pub fn openclaw_response_to_a2a(original_id: Uuid, frame: &ResponseFrame) -> A2aResponse {
    if let Some(error) = &frame.error {
        A2aResponse::Error {
            jsonrpc: "2.0".to_string(),
            id: original_id,
            error: JsonRpcError {
                code: error.code,
                message: error.message.clone(),
                data: error.data.clone(),
            },
        }
    } else {
        A2aResponse::Success {
            jsonrpc: "2.0".to_string(),
            id: original_id,
            result: frame.result.clone().unwrap_or(Value::Null),
        }
    }
}

fn extract_content(params: Option<&Value>) -> Result<String, TranslatorError> {
    let params = params.ok_or(TranslatorError::MissingParams)?;
    // Try common OpenClaw patterns: {content: "..."}, {message: "..."}, or plain string
    if let Some(content) = params.get("content").and_then(Value::as_str) {
        return Ok(content.to_string());
    }
    if let Some(message) = params.get("message").and_then(Value::as_str) {
        return Ok(message.to_string());
    }
    if let Some(s) = params.as_str() {
        return Ok(s.to_string());
    }
    // Fall back to stringified params
    Ok(params.to_string())
}

#[derive(Debug, thiserror::Error)]
pub enum TranslatorError {
    #[error("Missing params in request frame")]
    MissingParams,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_request_maps_to_message_send() {
        let frame = RequestFrame {
            id: "r1".to_string(),
            method: "chat".to_string(),
            params: Some(serde_json::json!({"content": "hello world"})),
        };
        let a2a = openclaw_req_to_a2a(&frame).unwrap();
        assert_eq!(a2a.method, "message/send");
        let parts = &a2a.params["request"]["message"]["parts"];
        assert_eq!(parts[0]["content"], "hello world");
    }

    #[test]
    fn send_message_alias() {
        let frame = RequestFrame {
            id: "r2".to_string(),
            method: "send_message".to_string(),
            params: Some(serde_json::json!({"message": "hi"})),
        };
        let a2a = openclaw_req_to_a2a(&frame).unwrap();
        assert_eq!(a2a.method, "message/send");
    }

    #[test]
    fn discover_request_maps_to_agent_discover() {
        let frame = RequestFrame {
            id: "r3".to_string(),
            method: "discover".to_string(),
            params: None,
        };
        let a2a = openclaw_req_to_a2a(&frame).unwrap();
        assert_eq!(a2a.method, "agent_discover");
    }

    #[test]
    fn status_request_maps_to_tasks_get() {
        let frame = RequestFrame {
            id: "r4".to_string(),
            method: "status".to_string(),
            params: Some(serde_json::json!({"task_id": "t1"})),
        };
        let a2a = openclaw_req_to_a2a(&frame).unwrap();
        assert_eq!(a2a.method, "tasks/get");
    }

    #[test]
    fn cancel_request_maps_to_tasks_cancel() {
        let frame = RequestFrame {
            id: "r5".to_string(),
            method: "cancel".to_string(),
            params: Some(serde_json::json!({"task_id": "t1"})),
        };
        let a2a = openclaw_req_to_a2a(&frame).unwrap();
        assert_eq!(a2a.method, "tasks/cancel");
    }

    #[test]
    fn unknown_method_passes_through() {
        let frame = RequestFrame {
            id: "r6".to_string(),
            method: "custom.method".to_string(),
            params: Some(serde_json::json!({"key": "val"})),
        };
        let a2a = openclaw_req_to_a2a(&frame).unwrap();
        assert_eq!(a2a.method, "custom.method");
    }

    #[test]
    fn success_response_translation() {
        let a2a = A2aResponse::Success {
            jsonrpc: "2.0".to_string(),
            id: Uuid::new_v4(),
            result: serde_json::json!({"answer": 42}),
        };
        let oc = a2a_response_to_openclaw("r1", &a2a);
        assert_eq!(oc.id, "r1");
        assert!(oc.error.is_none());
        assert_eq!(oc.result.unwrap()["answer"], 42);
    }

    #[test]
    fn error_response_translation() {
        let a2a = A2aResponse::Error {
            jsonrpc: "2.0".to_string(),
            id: Uuid::new_v4(),
            error: JsonRpcError {
                code: -32601,
                message: "Method not found".to_string(),
                data: None,
            },
        };
        let oc = a2a_response_to_openclaw("r2", &a2a);
        assert_eq!(oc.id, "r2");
        assert!(oc.result.is_none());
        let err = oc.error.unwrap();
        assert_eq!(err.code, -32601);
    }

    #[test]
    fn a2a_to_openclaw_event_chat_stream() {
        let ev = a2a_event_to_openclaw("chat_stream", serde_json::json!({"text": "hi"}));
        assert_eq!(ev.topic, "chat.delta");
    }

    #[test]
    fn a2a_to_openclaw_event_broadcast() {
        let ev = a2a_event_to_openclaw("agent_broadcast", serde_json::json!({"status": "ready"}));
        assert_eq!(ev.topic, "status");
    }

    #[test]
    fn a2a_to_openclaw_req_message_send() {
        let req = A2aRequest::new("message/send", serde_json::json!({"content": "test"}));
        let frame = a2a_to_openclaw_req(&req);
        assert_eq!(frame.method, "chat");
        assert_eq!(frame.id, req.id.to_string());
    }

    #[test]
    fn openclaw_response_to_a2a_success() {
        let id = Uuid::new_v4();
        let frame = ResponseFrame {
            id: "oc-1".to_string(),
            result: Some(serde_json::json!({"ok": true})),
            error: None,
        };
        let resp = openclaw_response_to_a2a(id, &frame);
        match resp {
            A2aResponse::Success {
                id: rid, result, ..
            } => {
                assert_eq!(rid, id);
                assert_eq!(result["ok"], true);
            }
            _ => panic!("expected success"),
        }
    }

    #[test]
    fn openclaw_response_to_a2a_error() {
        let id = Uuid::new_v4();
        let frame = ResponseFrame {
            id: "oc-2".to_string(),
            result: None,
            error: Some(OpenClawError {
                code: -1,
                message: "fail".to_string(),
                data: None,
            }),
        };
        let resp = openclaw_response_to_a2a(id, &frame);
        match resp {
            A2aResponse::Error { error, .. } => {
                assert_eq!(error.code, -1);
                assert_eq!(error.message, "fail");
            }
            _ => panic!("expected error"),
        }
    }

    #[test]
    fn task_request_wraps_in_message_send() {
        let frame = RequestFrame {
            id: "r7".to_string(),
            method: "task".to_string(),
            params: Some(serde_json::json!({"description": "do stuff"})),
        };
        let a2a = openclaw_req_to_a2a(&frame).unwrap();
        assert_eq!(a2a.method, "message/send");
        assert!(a2a.params["request"]["metadata"]["source"] == "openclaw");
    }
}
