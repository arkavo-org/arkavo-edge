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
        "chat.send" | "send" | "chat" | "send_message" => {
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
        "agents.list" | "discover" | "list_agents" => {
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
        "cancel" | "chat.abort" => {
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
            ok: true,
            payload: Some(result.clone()),
            error: None,
        },
        A2aResponse::Error { error, .. } => ResponseFrame {
            id: openclaw_id.to_string(),
            ok: false,
            payload: None,
            error: Some(OpenClawError {
                code: a2a_error_code_to_string(error.code),
                message: error.message.clone(),
                data: error.data.clone(),
            }),
        },
    }
}

/// Translate an A2A event into an OpenClaw event frame.
pub fn a2a_event_to_openclaw(method: &str, data: Value) -> EventFrame {
    let event_name = match method {
        "message/stream" | "chat_stream" => "chat",
        "agent_broadcast" => "presence",
        other => other,
    };
    EventFrame {
        event: event_name.to_string(),
        payload: data,
        seq: None,
        state_version: None,
    }
}

/// Translate an A2A JSON-RPC request into an OpenClaw request frame (outbound).
pub fn a2a_to_openclaw_req(request: &A2aRequest) -> RequestFrame {
    let method = match request.method.as_str() {
        "message/send" => "chat.send",
        "agent_discover" => "agents.list",
        "tasks/get" => "status",
        "tasks/cancel" => "chat.abort",
        other => other,
    };
    RequestFrame {
        id: request.id.to_string(),
        method: method.to_string(),
        params: Some(request.params.clone()),
    }
}

/// Translate an OpenClaw response frame back into an A2A JSON-RPC response.
pub fn openclaw_response_to_a2a(original_id: Uuid, frame: &ResponseFrame) -> A2aResponse {
    if !frame.ok {
        let (code, message) = frame
            .error
            .as_ref()
            .map(|e| (openclaw_error_code_to_int(&e.code), e.message.clone()))
            .unwrap_or_else(|| (-32000, "unknown error".to_string()));
        A2aResponse::Error {
            jsonrpc: "2.0".to_string(),
            id: original_id,
            error: JsonRpcError {
                code,
                message,
                data: frame.error.as_ref().and_then(|e| e.data.clone()),
            },
        }
    } else {
        A2aResponse::Success {
            jsonrpc: "2.0".to_string(),
            id: original_id,
            result: frame.payload.clone().unwrap_or(Value::Null),
        }
    }
}

fn extract_content(params: Option<&Value>) -> Result<String, TranslatorError> {
    let params = params.ok_or(TranslatorError::MissingParams)?;
    // OpenClaw chat.send uses params.message for the text
    if let Some(message) = params.get("message").and_then(Value::as_str) {
        return Ok(message.to_string());
    }
    if let Some(content) = params.get("content").and_then(Value::as_str) {
        return Ok(content.to_string());
    }
    if let Some(s) = params.as_str() {
        return Ok(s.to_string());
    }
    Ok(params.to_string())
}

fn a2a_error_code_to_string(code: i32) -> String {
    match code {
        -32700 => "PARSE_ERROR".to_string(),
        -32600 => "INVALID_REQUEST".to_string(),
        -32601 => "METHOD_NOT_FOUND".to_string(),
        -32602 => "INVALID_PARAMS".to_string(),
        -32603 => "INTERNAL_ERROR".to_string(),
        other => format!("ERROR_{other}"),
    }
}

fn openclaw_error_code_to_int(code: &str) -> i32 {
    match code {
        "PARSE_ERROR" => -32700,
        "INVALID_REQUEST" => -32600,
        "METHOD_NOT_FOUND" => -32601,
        "INVALID_PARAMS" => -32602,
        "INTERNAL_ERROR" => -32603,
        "NOT_LINKED" => -32001,
        _ => -32000,
    }
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
    fn chat_send_request_maps_to_message_send() {
        let frame = RequestFrame {
            id: "r1".to_string(),
            method: "chat.send".to_string(),
            params: Some(serde_json::json!({"message": "hello world"})),
        };
        let a2a = openclaw_req_to_a2a(&frame).unwrap();
        assert_eq!(a2a.method, "message/send");
        let parts = &a2a.params["request"]["message"]["parts"];
        assert_eq!(parts[0]["content"], "hello world");
    }

    #[test]
    fn chat_alias_maps_to_message_send() {
        let frame = RequestFrame {
            id: "r2".to_string(),
            method: "chat".to_string(),
            params: Some(serde_json::json!({"content": "hi"})),
        };
        let a2a = openclaw_req_to_a2a(&frame).unwrap();
        assert_eq!(a2a.method, "message/send");
    }

    #[test]
    fn agents_list_maps_to_agent_discover() {
        let frame = RequestFrame {
            id: "r3".to_string(),
            method: "agents.list".to_string(),
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
    fn chat_abort_maps_to_tasks_cancel() {
        let frame = RequestFrame {
            id: "r5".to_string(),
            method: "chat.abort".to_string(),
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
        assert!(oc.ok);
        assert_eq!(oc.payload.unwrap()["answer"], 42);
        assert!(oc.error.is_none());
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
        assert!(!oc.ok);
        assert!(oc.payload.is_none());
        let err = oc.error.unwrap();
        assert_eq!(err.code, "METHOD_NOT_FOUND");
    }

    #[test]
    fn a2a_to_openclaw_event_chat_stream() {
        let ev = a2a_event_to_openclaw("chat_stream", serde_json::json!({"text": "hi"}));
        assert_eq!(ev.event, "chat");
    }

    #[test]
    fn a2a_to_openclaw_event_broadcast() {
        let ev = a2a_event_to_openclaw("agent_broadcast", serde_json::json!({"status": "ready"}));
        assert_eq!(ev.event, "presence");
    }

    #[test]
    fn a2a_to_openclaw_req_message_send() {
        let req = A2aRequest::new("message/send", serde_json::json!({"content": "test"}));
        let frame = a2a_to_openclaw_req(&req);
        assert_eq!(frame.method, "chat.send");
        assert_eq!(frame.id, req.id.to_string());
    }

    #[test]
    fn openclaw_response_to_a2a_success() {
        let id = Uuid::new_v4();
        let frame = ResponseFrame {
            id: "oc-1".to_string(),
            ok: true,
            payload: Some(serde_json::json!({"ok": true})),
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
            ok: false,
            payload: None,
            error: Some(OpenClawError {
                code: "INVALID_REQUEST".to_string(),
                message: "fail".to_string(),
                data: None,
            }),
        };
        let resp = openclaw_response_to_a2a(id, &frame);
        match resp {
            A2aResponse::Error { error, .. } => {
                assert_eq!(error.code, -32600);
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

    #[test]
    fn error_code_round_trip() {
        assert_eq!(a2a_error_code_to_string(-32601), "METHOD_NOT_FOUND");
        assert_eq!(openclaw_error_code_to_int("METHOD_NOT_FOUND"), -32601);
        assert_eq!(a2a_error_code_to_string(-32600), "INVALID_REQUEST");
        assert_eq!(openclaw_error_code_to_int("INVALID_REQUEST"), -32600);
    }
}
