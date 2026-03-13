//! Message processing for hook event extraction

use crate::error::Result;
use crate::types::{ContentBlock, HookContext, HookEvent, HookOutput, Message};

use super::{HookManager, PendingToolUse, content_to_string};

impl HookManager {
    /// Process a message and invoke applicable hooks
    ///
    /// This method extracts hook events from messages and invokes registered
    /// hooks. It tracks tool uses to match them with results.
    ///
    /// # Events extracted:
    /// - `ToolUse` in Assistant message -> `PreToolUse` (+ `SubagentStart` for Task)
    /// - `ToolResult` in User message -> `PostToolUse` or `PostToolUseFailure` (+ `SubagentStop` for Task)
    /// - `Result` message -> captures `session_id`
    ///
    /// # Returns
    /// Vector of hook outputs from all invoked hooks
    ///
    /// # Errors
    ///
    /// Propagates errors from hook invocation.
    #[allow(clippy::too_many_lines)]
    pub async fn process_message(&mut self, msg: &Message) -> Result<Vec<HookOutput>> {
        let mut outputs = Vec::new();
        let context = self.build_context();

        // Extract parent_tool_use_id from message for subagent tracking
        let parent_tool_use_id = match msg {
            Message::Assistant {
                parent_tool_use_id, ..
            }
            | Message::User {
                parent_tool_use_id, ..
            } => parent_tool_use_id.clone(),
            _ => None,
        };

        // Check for SubagentStop: if we were inside a subagent and now we're not
        if let Some(ref current) = self.current_subagent
            && parent_tool_use_id.is_none()
        {
            // We've exited subagent context - trigger SubagentStop
            let agent_id = current.clone();

            let subagent_input = serde_json::json!({
                "hook_event_name": "SubagentStop",
                "session_id": self.session_id.as_deref().unwrap_or(""),
                "cwd": self.cwd.as_deref().unwrap_or(""),
                "transcript_path": "",
                "stop_hook_active": false,
                "agent_id": agent_id,
                "agent_transcript_path": "",
            });

            let output = self
                .invoke(
                    HookEvent::SubagentStop,
                    subagent_input,
                    Some("Task".to_string()),
                    context.clone(),
                )
                .await?;

            if output.decision.is_some()
                || output.system_message.is_some()
                || output.hook_specific_output.is_some()
            {
                outputs.push(output);
            }

            // Clear subagent context and remove from pending
            self.pending_tools.remove(&agent_id);
            self.current_subagent = None;
        }

        // Update subagent context tracking
        if let Some(ref ptui) = parent_tool_use_id {
            // Check if this is from a Task tool (subagent)
            if self.pending_tools.get(ptui).is_some_and(|p| p.is_task) {
                self.current_subagent = Some(ptui.clone());
            }
        }

        match msg {
            Message::Assistant { message, .. } => {
                self.process_assistant_message(message, &context, &mut outputs)
                    .await?;
            }

            Message::User { message, .. } => {
                self.process_user_message(message, &context, &mut outputs)
                    .await?;
            }

            Message::Result { session_id, .. } => {
                self.process_result_message(session_id, &context, &mut outputs)
                    .await?;
            }

            Message::System { subtype, data } => {
                self.process_system_message(subtype, data, &context, &mut outputs)
                    .await?;
            }

            Message::StreamEvent { .. } => {}
        }

        Ok(outputs)
    }

    async fn process_assistant_message(
        &mut self,
        message: &crate::types::AssistantMessageContent,
        context: &HookContext,
        outputs: &mut Vec<HookOutput>,
    ) -> Result<()> {
        // Extract ToolUse blocks
        for block in &message.content {
            if let ContentBlock::ToolUse { id, name, input } = block {
                // Track for later PostToolUse matching
                self.pending_tools.insert(
                    id.clone(),
                    PendingToolUse {
                        tool_name: name.clone(),
                        tool_input: input.clone(),
                        is_task: name == "Task",
                    },
                );

                // Build PreToolUse input
                let hook_input = serde_json::json!({
                    "hook_event_name": "PreToolUse",
                    "session_id": self.session_id.as_deref().unwrap_or(""),
                    "cwd": self.cwd.as_deref().unwrap_or(""),
                    "transcript_path": "",
                    "tool_name": name,
                    "tool_input": input,
                });

                // Invoke PreToolUse hooks
                let output = self
                    .invoke(
                        HookEvent::PreToolUse,
                        hook_input,
                        Some(name.clone()),
                        context.clone(),
                    )
                    .await?;

                if output.decision.is_some()
                    || output.system_message.is_some()
                    || output.hook_specific_output.is_some()
                {
                    outputs.push(output);
                }

                // For Task tool, also invoke SubagentStart
                if name == "Task" {
                    let subagent_input = serde_json::json!({
                        "hook_event_name": "SubagentStart",
                        "session_id": self.session_id.as_deref().unwrap_or(""),
                        "cwd": self.cwd.as_deref().unwrap_or(""),
                        "transcript_path": "",
                        "agent_id": id,
                        "agent_type": input.get("subagent_type").and_then(|v| v.as_str()).unwrap_or("unknown"),
                    });

                    let output = self
                        .invoke(
                            HookEvent::SubagentStart,
                            subagent_input,
                            Some(name.clone()),
                            context.clone(),
                        )
                        .await?;

                    if output.decision.is_some()
                        || output.system_message.is_some()
                        || output.hook_specific_output.is_some()
                    {
                        outputs.push(output);
                    }
                }
            }
        }
        Ok(())
    }

    async fn process_user_message(
        &mut self,
        message: &crate::types::UserMessageContent,
        context: &HookContext,
        outputs: &mut Vec<HookOutput>,
    ) -> Result<()> {
        // Check for ToolResult in content blocks
        if let Some(crate::types::UserContent::Blocks(blocks)) = &message.content {
            for block in blocks {
                if let ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } = block
                {
                    // Look up pending tool use
                    if let Some(pending) = self.pending_tools.remove(tool_use_id.as_str()) {
                        let is_failure = is_error.unwrap_or(false);

                        if is_failure {
                            self.process_tool_failure(
                                &pending,
                                tool_use_id,
                                content.as_ref(),
                                context,
                                outputs,
                            )
                            .await?;
                        } else {
                            self.process_tool_success(
                                &pending,
                                tool_use_id,
                                content.as_ref(),
                                context,
                                outputs,
                            )
                            .await?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn process_tool_failure(
        &self,
        pending: &PendingToolUse,
        tool_use_id: &str,
        content: Option<&crate::types::ContentValue>,
        context: &HookContext,
        outputs: &mut Vec<HookOutput>,
    ) -> Result<()> {
        let hook_input = serde_json::json!({
            "hook_event_name": "PostToolUseFailure",
            "session_id": self.session_id.as_deref().unwrap_or(""),
            "cwd": self.cwd.as_deref().unwrap_or(""),
            "transcript_path": "",
            "tool_name": pending.tool_name,
            "tool_input": pending.tool_input,
            "tool_use_id": tool_use_id,
            "error": content_to_string(content),
        });

        let output = self
            .invoke(
                HookEvent::PostToolUseFailure,
                hook_input,
                Some(pending.tool_name.clone()),
                context.clone(),
            )
            .await?;

        if output.decision.is_some()
            || output.system_message.is_some()
            || output.hook_specific_output.is_some()
        {
            outputs.push(output);
        }
        Ok(())
    }

    async fn process_tool_success(
        &self,
        pending: &PendingToolUse,
        tool_use_id: &str,
        content: Option<&crate::types::ContentValue>,
        context: &HookContext,
        outputs: &mut Vec<HookOutput>,
    ) -> Result<()> {
        let hook_input = serde_json::json!({
            "hook_event_name": "PostToolUse",
            "session_id": self.session_id.as_deref().unwrap_or(""),
            "cwd": self.cwd.as_deref().unwrap_or(""),
            "transcript_path": "",
            "tool_name": pending.tool_name,
            "tool_input": pending.tool_input,
            "tool_response": content,
            "tool_use_id": tool_use_id,
        });

        let output = self
            .invoke(
                HookEvent::PostToolUse,
                hook_input,
                Some(pending.tool_name.clone()),
                context.clone(),
            )
            .await?;

        if output.decision.is_some()
            || output.system_message.is_some()
            || output.hook_specific_output.is_some()
        {
            outputs.push(output);
        }

        // For Task tool, also invoke SubagentStop
        if pending.is_task {
            let subagent_input = serde_json::json!({
                "hook_event_name": "SubagentStop",
                "session_id": self.session_id.as_deref().unwrap_or(""),
                "cwd": self.cwd.as_deref().unwrap_or(""),
                "transcript_path": "",
                "stop_hook_active": false,
                "agent_id": tool_use_id,
                "agent_transcript_path": "",
            });

            let output = self
                .invoke(
                    HookEvent::SubagentStop,
                    subagent_input,
                    Some(pending.tool_name.clone()),
                    context.clone(),
                )
                .await?;

            if output.decision.is_some()
                || output.system_message.is_some()
                || output.hook_specific_output.is_some()
            {
                outputs.push(output);
            }
        }
        Ok(())
    }

    async fn process_result_message(
        &mut self,
        session_id: &str,
        context: &HookContext,
        outputs: &mut Vec<HookOutput>,
    ) -> Result<()> {
        // Capture session_id for context
        self.session_id = Some(session_id.to_string());

        // Trigger Stop hook
        let stop_input = serde_json::json!({
            "hook_event_name": "Stop",
            "session_id": self.session_id.as_deref().unwrap_or(""),
            "cwd": self.cwd.as_deref().unwrap_or(""),
            "transcript_path": "",
            "stop_hook_active": false,
        });

        let output = self
            .invoke(HookEvent::Stop, stop_input, None, context.clone())
            .await?;

        if output.decision.is_some()
            || output.system_message.is_some()
            || output.hook_specific_output.is_some()
        {
            outputs.push(output);
        }
        Ok(())
    }

    async fn process_system_message(
        &self,
        subtype: &str,
        data: &serde_json::Value,
        context: &HookContext,
        outputs: &mut Vec<HookOutput>,
    ) -> Result<()> {
        // Handle compact_boundary for PreCompact hook
        if subtype == "compact_boundary" {
            let trigger = data
                .get("compact_metadata")
                .and_then(|m| m.get("trigger"))
                .and_then(|t| t.as_str())
                .unwrap_or("auto");

            let precompact_input = serde_json::json!({
                "hook_event_name": "PreCompact",
                "session_id": self.session_id.as_deref().unwrap_or(""),
                "cwd": self.cwd.as_deref().unwrap_or(""),
                "transcript_path": "",
                "trigger": trigger,
                "custom_instructions": null,
            });

            let output = self
                .invoke(
                    HookEvent::PreCompact,
                    precompact_input,
                    None,
                    context.clone(),
                )
                .await?;

            if output.decision.is_some()
                || output.system_message.is_some()
                || output.hook_specific_output.is_some()
            {
                outputs.push(output);
            }
        }
        Ok(())
    }
}
