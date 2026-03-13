use arkavo_events::payload::UsageInfo;
use arkavo_events::{Event, EventPayload};

use super::EventMapper;

impl EventMapper {
    /// Emit a run started event
    pub async fn emit_run_started(&self, run_id: &str, prompt: &str) {
        let sequence = self
            .sequence_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let event = Event::new(
            run_id.to_string(),
            sequence,
            self.agent_id.clone(),
            EventPayload::PromptSent {
                prompt: prompt.to_string(),
                model: "claude-code".to_string(),
                parameters: None,
            },
        );

        if let Err(e) = self.event_writer.write(event).await {
            tracing::error!("Failed to write run started event: {}", e);
        }
    }

    /// Emit a run completed event
    pub async fn emit_run_completed(&self, run_id: &str) {
        let sequence = self
            .sequence_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let event = Event::new(
            run_id.to_string(),
            sequence,
            self.agent_id.clone(),
            EventPayload::SessionEnded {
                reason: "completed".to_string(),
                duration_ms: 0,
                summary: None,
            },
        );

        if let Err(e) = self.event_writer.write(event).await {
            tracing::error!("Failed to write run completed event: {}", e);
        }
    }

    /// Emit an assistant message event
    pub async fn emit_assistant_message(&self, run_id: &str, content: &str) {
        let sequence = self
            .sequence_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let event = Event::new(
            run_id.to_string(),
            sequence,
            self.agent_id.clone(),
            EventPayload::StreamDelta {
                stream_id: run_id.to_string(),
                sequence,
                delta_type: "assistant".to_string(),
                content: content.to_string(),
            },
        );

        if let Err(e) = self.event_writer.write(event).await {
            tracing::error!("Failed to write assistant message event: {}", e);
        }
    }

    /// Emit a system message event
    pub async fn emit_system_message(&self, run_id: &str, content: &str) {
        let sequence = self
            .sequence_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let event = Event::new(
            run_id.to_string(),
            sequence,
            self.agent_id.clone(),
            EventPayload::StreamDelta {
                stream_id: run_id.to_string(),
                sequence,
                delta_type: "system".to_string(),
                content: content.to_string(),
            },
        );

        if let Err(e) = self.event_writer.write(event).await {
            tracing::error!("Failed to write system message event: {}", e);
        }
    }

    /// Emit an error event
    pub async fn emit_error(&self, run_id: &str, error: &str) {
        let sequence = self
            .sequence_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let event = Event::new(
            run_id.to_string(),
            sequence,
            self.agent_id.clone(),
            EventPayload::Error {
                error_type: "sdk_error".to_string(),
                message: error.to_string(),
                stack_trace: None,
                recoverable: Some(true),
            },
        );

        if let Err(e) = self.event_writer.write(event).await {
            tracing::error!("Failed to write error event: {}", e);
        }
    }

    /// Emit a typed tool result event from SDK hooks
    pub async fn emit_tool_result_typed(
        &self,
        run_id: &str,
        tool_name: &str,
        success: bool,
        output: &serde_json::Value,
        duration_ms: u64,
    ) {
        let sequence = self
            .sequence_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let event = Event::new(
            run_id.to_string(),
            sequence,
            self.agent_id.clone(),
            EventPayload::ToolResult {
                tool_name: tool_name.to_string(),
                tool_call_id: None,
                success,
                result: output.clone(),
                duration_ms,
            },
        );

        if let Err(e) = self.event_writer.write(event).await {
            tracing::error!("Failed to write tool result event: {}", e);
        }
    }

    /// Emit a permission decision event for UI visibility
    pub async fn emit_permission_decision(
        &self,
        run_id: &str,
        tool_name: &str,
        allowed: bool,
        reason: &str,
    ) {
        let sequence = self
            .sequence_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let event = Event::new(
            run_id.to_string(),
            sequence,
            self.agent_id.clone(),
            EventPayload::ReasoningStep {
                step_type: "permission".to_string(),
                description: format!(
                    "Tool '{tool_name}' {}: {reason}",
                    if allowed { "allowed" } else { "denied" }
                ),
                metadata: Some(serde_json::json!({
                    "tool_name": tool_name,
                    "allowed": allowed,
                    "reason": reason,
                })),
            },
        );

        if let Err(e) = self.event_writer.write(event).await {
            tracing::error!("Failed to write permission decision event: {}", e);
        }
    }

    /// Emit budget/metrics from SDK `Message::Result`
    pub async fn emit_run_metrics(
        &self,
        run_id: &str,
        duration_ms: u64,
        num_turns: u32,
        total_cost_usd: Option<f64>,
    ) {
        let sequence = self
            .sequence_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let event = Event::new(
            run_id.to_string(),
            sequence,
            self.agent_id.clone(),
            EventPayload::ModelResponse {
                model: "claude-code".to_string(),
                response: format!("Completed in {num_turns} turns"),
                usage: total_cost_usd.map(|_| UsageInfo {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                }),
                duration_ms,
            },
        );

        if let Err(e) = self.event_writer.write(event).await {
            tracing::error!("Failed to write run metrics event: {}", e);
        }
    }

    /// Emit a result event
    pub async fn emit_result(&self, run_id: &str, result: &str) {
        let sequence = self
            .sequence_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let event = Event::new(
            run_id.to_string(),
            sequence,
            self.agent_id.clone(),
            EventPayload::ModelResponse {
                model: "claude-code".to_string(),
                response: result.to_string(),
                usage: None,
                duration_ms: 0,
            },
        );

        if let Err(e) = self.event_writer.write(event).await {
            tracing::error!("Failed to write result event: {}", e);
        }
    }
}
