use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventPayload {
    PromptSent {
        prompt: String,
        model: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        parameters: Option<HashMap<String, Value>>,
    },
    ModelResponse {
        model: String,
        response: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<UsageInfo>,
        duration_ms: u64,
    },
    ToolCall {
        tool_name: String,
        parameters: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_call_id: Option<String>,
    },
    ToolResult {
        tool_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_call_id: Option<String>,
        success: bool,
        result: Value,
        duration_ms: u64,
    },
    FileOperation {
        operation: FileOp,
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        content_preview: Option<String>,
        success: bool,
    },
    ReasoningStep {
        step_type: String,
        description: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<Value>,
    },
    StreamDelta {
        stream_id: String,
        sequence: u64,
        delta_type: String,
        content: String,
    },
    Error {
        error_type: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        stack_trace: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        recoverable: Option<bool>,
    },
    SessionStarted {
        #[serde(skip_serializing_if = "Option::is_none")]
        capabilities: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, Value>>,
    },
    SessionEnded {
        reason: String,
        duration_ms: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<SessionSummary>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageInfo {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOp {
    Read,
    Write,
    Edit,
    Delete,
    Create,
    Rename,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub total_events: u64,
    pub total_prompts: u32,
    pub total_tool_calls: u32,
    pub total_errors: u32,
    pub total_tokens_used: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn roundtrip(payload: &EventPayload) -> EventPayload {
        let json = serde_json::to_string(payload).expect("serialize");
        serde_json::from_str(&json).expect("deserialize")
    }

    #[test]
    fn roundtrip_prompt_sent() {
        let mut params = HashMap::new();
        params.insert("temperature".to_string(), serde_json::json!(0.7));
        let payload = EventPayload::PromptSent {
            prompt: "hello world".to_string(),
            model: "test-model".to_string(),
            parameters: Some(params),
        };
        let restored = roundtrip(&payload);
        match restored {
            EventPayload::PromptSent {
                prompt,
                model,
                parameters,
            } => {
                assert_eq!(prompt, "hello world");
                assert_eq!(model, "test-model");
                assert!(parameters.is_some());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_prompt_sent_no_params() {
        let payload = EventPayload::PromptSent {
            prompt: "hi".to_string(),
            model: "m".to_string(),
            parameters: None,
        };
        let restored = roundtrip(&payload);
        match restored {
            EventPayload::PromptSent { parameters, .. } => {
                assert!(parameters.is_none());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_model_response() {
        let payload = EventPayload::ModelResponse {
            model: "gpt-4".to_string(),
            response: "answer".to_string(),
            usage: Some(UsageInfo {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            }),
            duration_ms: 500,
        };
        let restored = roundtrip(&payload);
        match restored {
            EventPayload::ModelResponse {
                model,
                response,
                usage,
                duration_ms,
            } => {
                assert_eq!(model, "gpt-4");
                assert_eq!(response, "answer");
                assert_eq!(duration_ms, 500);
                let u = usage.expect("usage present");
                assert_eq!(u.total_tokens, 30);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_tool_call() {
        let payload = EventPayload::ToolCall {
            tool_name: "read_file".to_string(),
            parameters: serde_json::json!({"path": "/tmp/a.txt"}),
            tool_call_id: Some("tc-1".to_string()),
        };
        let restored = roundtrip(&payload);
        match restored {
            EventPayload::ToolCall {
                tool_name,
                parameters,
                tool_call_id,
            } => {
                assert_eq!(tool_name, "read_file");
                assert_eq!(parameters["path"], "/tmp/a.txt");
                assert_eq!(tool_call_id, Some("tc-1".to_string()));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_tool_result() {
        let payload = EventPayload::ToolResult {
            tool_name: "read_file".to_string(),
            tool_call_id: None,
            success: true,
            result: serde_json::json!("file contents"),
            duration_ms: 12,
        };
        let restored = roundtrip(&payload);
        match restored {
            EventPayload::ToolResult {
                success,
                duration_ms,
                ..
            } => {
                assert!(success);
                assert_eq!(duration_ms, 12);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_file_operation() {
        let payload = EventPayload::FileOperation {
            operation: FileOp::Write,
            path: "/tmp/out.txt".to_string(),
            content_preview: Some("preview...".to_string()),
            success: true,
        };
        let restored = roundtrip(&payload);
        match restored {
            EventPayload::FileOperation {
                operation,
                path,
                content_preview,
                success,
            } => {
                assert!(matches!(operation, FileOp::Write));
                assert_eq!(path, "/tmp/out.txt");
                assert_eq!(content_preview, Some("preview...".to_string()));
                assert!(success);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_file_op_all_variants() {
        let ops = [
            FileOp::Read,
            FileOp::Write,
            FileOp::Edit,
            FileOp::Delete,
            FileOp::Create,
            FileOp::Rename,
        ];
        for op in &ops {
            let payload = EventPayload::FileOperation {
                operation: op.clone(),
                path: "p".to_string(),
                content_preview: None,
                success: false,
            };
            let json = serde_json::to_string(&payload).expect("serialize");
            let _: EventPayload = serde_json::from_str(&json).expect("deserialize");
        }
    }

    #[test]
    fn roundtrip_reasoning_step() {
        let payload = EventPayload::ReasoningStep {
            step_type: "analysis".to_string(),
            description: "thinking about it".to_string(),
            metadata: Some(serde_json::json!({"depth": 3})),
        };
        let restored = roundtrip(&payload);
        match restored {
            EventPayload::ReasoningStep {
                step_type,
                description,
                metadata,
            } => {
                assert_eq!(step_type, "analysis");
                assert_eq!(description, "thinking about it");
                assert_eq!(metadata.unwrap()["depth"], 3);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_stream_delta() {
        let payload = EventPayload::StreamDelta {
            stream_id: "stream-1".to_string(),
            sequence: 42,
            delta_type: "text".to_string(),
            content: "chunk".to_string(),
        };
        let restored = roundtrip(&payload);
        match restored {
            EventPayload::StreamDelta {
                stream_id,
                sequence,
                delta_type,
                content,
            } => {
                assert_eq!(stream_id, "stream-1");
                assert_eq!(sequence, 42);
                assert_eq!(delta_type, "text");
                assert_eq!(content, "chunk");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_error() {
        let payload = EventPayload::Error {
            error_type: "timeout".to_string(),
            message: "request timed out".to_string(),
            stack_trace: Some("at line 42".to_string()),
            recoverable: Some(true),
        };
        let restored = roundtrip(&payload);
        match restored {
            EventPayload::Error {
                error_type,
                message,
                stack_trace,
                recoverable,
            } => {
                assert_eq!(error_type, "timeout");
                assert_eq!(message, "request timed out");
                assert_eq!(stack_trace, Some("at line 42".to_string()));
                assert_eq!(recoverable, Some(true));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_session_started() {
        let payload = EventPayload::SessionStarted {
            capabilities: Some(vec!["code".to_string(), "chat".to_string()]),
            metadata: None,
        };
        let restored = roundtrip(&payload);
        match restored {
            EventPayload::SessionStarted {
                capabilities,
                metadata,
            } => {
                let caps = capabilities.expect("capabilities present");
                assert_eq!(caps, vec!["code", "chat"]);
                assert!(metadata.is_none());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_session_ended() {
        let payload = EventPayload::SessionEnded {
            reason: "user_exit".to_string(),
            duration_ms: 30000,
            summary: Some(SessionSummary {
                total_events: 50,
                total_prompts: 10,
                total_tool_calls: 25,
                total_errors: 2,
                total_tokens_used: Some(5000),
            }),
        };
        let restored = roundtrip(&payload);
        match restored {
            EventPayload::SessionEnded {
                reason,
                duration_ms,
                summary,
            } => {
                assert_eq!(reason, "user_exit");
                assert_eq!(duration_ms, 30000);
                let s = summary.expect("summary present");
                assert_eq!(s.total_events, 50);
                assert_eq!(s.total_prompts, 10);
                assert_eq!(s.total_tool_calls, 25);
                assert_eq!(s.total_errors, 2);
                assert_eq!(s.total_tokens_used, Some(5000));
            }
            _ => panic!("wrong variant"),
        }
    }
}
