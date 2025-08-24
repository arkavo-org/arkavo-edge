use serde_json::Value;
use std::sync::Arc;
use tracing::{debug, info};

use arkavo_events::payload::{FileOp, UsageInfo};
use arkavo_events::{Event, EventPayload, EventWriter};

/// Maps Claude Code SDK events to Arkavo event types
pub struct EventMapper {
    agent_id: String,
    event_writer: Arc<EventWriter>,
    sequence_counter: std::sync::atomic::AtomicU64,
}

impl EventMapper {
    pub fn new(agent_id: String, event_writer: Arc<EventWriter>) -> Self {
        Self {
            agent_id,
            event_writer,
            sequence_counter: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Handle an event from the Claude Code SDK
    pub async fn handle_sdk_event(&self, event: Value) {
        let event_type = event["type"].as_str().unwrap_or("unknown");

        let session_id = event["data"]["sessionId"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();

        let sequence = self
            .sequence_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        debug!(
            "Mapping SDK event: {} for session: {}",
            event_type, session_id
        );

        let payload = match event_type {
            "plan_updated" => {
                event["data"]["plan"]
                    .as_object()
                    .map(|plan| EventPayload::ReasoningStep {
                        step_type: "plan".to_string(),
                        description: plan
                            .get("description")
                            .and_then(|d| d.as_str())
                            .unwrap_or("Plan updated")
                            .to_string(),
                        metadata: Some(event["data"]["plan"].clone()),
                    })
            }

            "tool_use" => event["data"]["tool"]
                .as_object()
                .map(|tool| EventPayload::ToolCall {
                    tool_name: tool
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    parameters: tool.get("parameters").cloned().unwrap_or(Value::Null),
                    tool_call_id: tool.get("id").and_then(|id| id.as_str()).map(String::from),
                }),

            "tool_result" => {
                event["data"]["result"]
                    .as_object()
                    .map(|result| EventPayload::ToolResult {
                        tool_name: result
                            .get("tool")
                            .and_then(|n| n.as_str())
                            .unwrap_or("unknown")
                            .to_string(),
                        tool_call_id: result
                            .get("id")
                            .and_then(|id| id.as_str())
                            .map(String::from),
                        success: result
                            .get("success")
                            .and_then(|s| s.as_bool())
                            .unwrap_or(false),
                        result: result.get("output").cloned().unwrap_or(Value::Null),
                        duration_ms: result
                            .get("duration_ms")
                            .and_then(|d| d.as_u64())
                            .unwrap_or(0),
                    })
            }

            "content_chunk" => {
                event["data"]["content"]
                    .as_str()
                    .map(|content| EventPayload::StreamDelta {
                        stream_id: session_id.clone(),
                        sequence,
                        delta_type: "content".to_string(),
                        content: content.to_string(),
                    })
            }

            "token_usage" => {
                if let Some(usage) = event["data"]["usage"].as_object() {
                    // We'll emit this as part of ModelResponse
                    info!(
                        "Token usage - prompt: {}, completion: {}, total: {}",
                        usage
                            .get("prompt_tokens")
                            .and_then(|t| t.as_u64())
                            .unwrap_or(0),
                        usage
                            .get("completion_tokens")
                            .and_then(|t| t.as_u64())
                            .unwrap_or(0),
                        usage
                            .get("total_tokens")
                            .and_then(|t| t.as_u64())
                            .unwrap_or(0)
                    );
                    None // Handled separately
                } else {
                    None
                }
            }

            "error" => event["data"]["error"]
                .as_str()
                .map(|error| EventPayload::Error {
                    error_type: "sdk_error".to_string(),
                    message: error.to_string(),
                    stack_trace: None,
                    recoverable: Some(true),
                }),

            "run_started" => {
                event["data"]["prompt"]
                    .as_str()
                    .map(|prompt| EventPayload::PromptSent {
                        prompt: prompt.to_string(),
                        model: "claude-code".to_string(),
                        parameters: None,
                    })
            }

            "run_completed" => {
                Some(EventPayload::ModelResponse {
                    model: "claude-code".to_string(),
                    response: "Task completed successfully".to_string(),
                    usage: None,    // Will be filled from token_usage events
                    duration_ms: 0, // TODO: Track actual duration
                })
            }

            "run_failed" => event["data"]["error"]
                .as_str()
                .map(|error| EventPayload::Error {
                    error_type: "run_failure".to_string(),
                    message: error.to_string(),
                    stack_trace: None,
                    recoverable: Some(false),
                }),

            "session_closed" => {
                Some(EventPayload::SessionEnded {
                    reason: "normal".to_string(),
                    duration_ms: 0, // TODO: Track actual duration
                    summary: None,
                })
            }

            _ => {
                debug!("Unmapped event type: {}", event_type);
                None
            }
        };

        // Write the event if we have a payload
        if let Some(payload) = payload {
            let event = Event::new(session_id, sequence, self.agent_id.clone(), payload);

            if let Err(e) = self.event_writer.write(event).await {
                tracing::error!("Failed to write event: {}", e);
            }
        }
    }

    /// Map file operation events
    pub async fn handle_file_operation(
        &self,
        session_id: String,
        operation: FileOp,
        path: String,
        success: bool,
        content_preview: Option<String>,
    ) {
        let sequence = self
            .sequence_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let event = Event::new(
            session_id,
            sequence,
            self.agent_id.clone(),
            EventPayload::FileOperation {
                operation,
                path,
                content_preview,
                success,
            },
        );

        if let Err(e) = self.event_writer.write(event).await {
            tracing::error!("Failed to write file operation event: {}", e);
        }
    }

    /// Track token usage for budget management
    pub async fn handle_token_usage(
        &self,
        session_id: String,
        prompt_tokens: u32,
        completion_tokens: u32,
    ) {
        let sequence = self
            .sequence_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let event = Event::new(
            session_id,
            sequence,
            self.agent_id.clone(),
            EventPayload::ModelResponse {
                model: "claude-code".to_string(),
                response: "".to_string(), // Empty for usage-only events
                usage: Some(UsageInfo {
                    prompt_tokens,
                    completion_tokens,
                    total_tokens: prompt_tokens + completion_tokens,
                }),
                duration_ms: 0,
            },
        );

        if let Err(e) = self.event_writer.write(event).await {
            tracing::error!("Failed to write token usage event: {}", e);
        }
    }
}
