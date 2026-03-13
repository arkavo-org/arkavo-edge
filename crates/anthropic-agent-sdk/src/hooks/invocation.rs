//! Hook invocation and explicit trigger methods

use crate::error::Result;
use crate::types::{HookContext, HookDecision, HookEvent, HookOutput};

use super::HookManager;

impl HookManager {
    /// Default timeout for hook callbacks (60 seconds, matching TypeScript SDK)
    pub const DEFAULT_HOOK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

    /// Invoke hooks for a specific event type
    ///
    /// # Arguments
    /// * `event` - The hook event type
    /// * `event_data` - Event data (JSON value matching `HookInput` structure)
    /// * `tool_name` - Optional tool name for filtering
    /// * `context` - Hook context
    ///
    /// # Returns
    /// Combined hook output from all matching hooks
    ///
    /// # Timeout Behavior
    /// Each hook matcher has a configurable timeout (default: 60 seconds).
    /// If a hook exceeds its timeout, it will be cancelled and a default
    /// `HookOutput` will be used, allowing the agent to continue.
    ///
    /// # Errors
    ///
    /// Propagates errors from hook callback execution.
    pub async fn invoke(
        &self,
        event: HookEvent,
        event_data: serde_json::Value,
        tool_name: Option<String>,
        context: HookContext,
    ) -> Result<HookOutput> {
        let mut output = HookOutput::default();

        // Get hooks for this event type
        let Some(matchers) = self.hooks_by_event.get(&event) else {
            return Ok(output);
        };

        // Find matching hooks
        for matcher in matchers {
            if Self::matches(matcher.matcher.as_ref(), tool_name.as_ref()) {
                let timeout = matcher.timeout.unwrap_or(Self::DEFAULT_HOOK_TIMEOUT);

                // Invoke each hook callback with timeout
                for hook in &matcher.hooks {
                    let hook_future =
                        hook.call(event_data.clone(), tool_name.clone(), context.clone());

                    let result = match tokio::time::timeout(timeout, hook_future).await {
                        Ok(hook_result) => hook_result?,
                        Err(_elapsed) => {
                            // Hook timed out - log warning and continue with default output
                            tracing::warn!(
                                event = ?event,
                                tool_name = ?tool_name,
                                timeout_secs = timeout.as_secs(),
                                "Hook callback timed out, continuing with default output"
                            );

                            // Continue with default output (don't block the agent)
                            HookOutput::default()
                        }
                    };

                    // Merge hook results
                    if result.decision.is_some() {
                        output.decision = result.decision;
                    }
                    if result.system_message.is_some() {
                        output.system_message = result.system_message;
                    }
                    if result.hook_specific_output.is_some() {
                        output.hook_specific_output = result.hook_specific_output;
                    }

                    // If decision is Block, stop processing
                    if matches!(output.decision, Some(HookDecision::Block)) {
                        return Ok(output);
                    }
                }
            }
        }

        Ok(output)
    }

    // ========================================================================
    // Explicit Hook Triggers
    // ========================================================================

    /// Trigger `SessionStart` hook
    ///
    /// Called when the session is initialized (after receiving system init message).
    ///
    /// # Arguments
    /// * `source` - Source of session start: "startup", "resume", "clear", or "compact"
    ///
    /// # Errors
    ///
    /// Propagates errors from hook callback execution.
    pub async fn trigger_session_start(&self, source: &str) -> Result<Vec<HookOutput>> {
        let mut outputs = Vec::new();
        let context = self.build_context();

        let session_start_input = serde_json::json!({
            "hook_event_name": "SessionStart",
            "session_id": self.session_id.as_deref().unwrap_or(""),
            "cwd": self.cwd.as_deref().unwrap_or(""),
            "transcript_path": "",
            "source": source,
        });

        let output = self
            .invoke(HookEvent::SessionStart, session_start_input, None, context)
            .await?;

        if output.decision.is_some()
            || output.system_message.is_some()
            || output.hook_specific_output.is_some()
        {
            outputs.push(output);
        }

        Ok(outputs)
    }

    /// Trigger `SessionEnd` hook
    ///
    /// Called when the session ends.
    ///
    /// # Arguments
    /// * `reason` - Reason for session end: "clear", "logout", "`prompt_input_exit`", or "other"
    ///
    /// # Errors
    ///
    /// Propagates errors from hook callback execution.
    pub async fn trigger_session_end(&self, reason: &str) -> Result<Vec<HookOutput>> {
        let mut outputs = Vec::new();
        let context = self.build_context();

        let session_end_input = serde_json::json!({
            "hook_event_name": "SessionEnd",
            "session_id": self.session_id.as_deref().unwrap_or(""),
            "cwd": self.cwd.as_deref().unwrap_or(""),
            "transcript_path": "",
            "reason": reason,
        });

        let output = self
            .invoke(HookEvent::SessionEnd, session_end_input, None, context)
            .await?;

        if output.decision.is_some()
            || output.system_message.is_some()
            || output.hook_specific_output.is_some()
        {
            outputs.push(output);
        }

        Ok(outputs)
    }

    /// Trigger `UserPromptSubmit` hook
    ///
    /// Called before a user message is sent to the agent.
    ///
    /// # Arguments
    /// * `prompt` - The user's prompt text
    ///
    /// # Returns
    /// Hook outputs that may contain `additionalContext` in `hook_specific_output`
    ///
    /// # Errors
    ///
    /// Propagates errors from hook callback execution.
    pub async fn trigger_user_prompt_submit(&self, prompt: &str) -> Result<Vec<HookOutput>> {
        let mut outputs = Vec::new();
        let context = self.build_context();

        let user_prompt_input = serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": self.session_id.as_deref().unwrap_or(""),
            "cwd": self.cwd.as_deref().unwrap_or(""),
            "transcript_path": "",
            "prompt": prompt,
        });

        let output = self
            .invoke(
                HookEvent::UserPromptSubmit,
                user_prompt_input,
                None,
                context,
            )
            .await?;

        if output.decision.is_some()
            || output.system_message.is_some()
            || output.hook_specific_output.is_some()
        {
            outputs.push(output);
        }

        Ok(outputs)
    }

    /// Trigger Notification hook
    ///
    /// Called when a notification is received.
    ///
    /// # Arguments
    /// * `message` - The notification message
    /// * `title` - Optional notification title
    ///
    /// # Errors
    ///
    /// Propagates errors from hook callback execution.
    pub async fn trigger_notification(
        &self,
        message: &str,
        title: Option<&str>,
    ) -> Result<Vec<HookOutput>> {
        let mut outputs = Vec::new();
        let context = self.build_context();

        let notification_input = serde_json::json!({
            "hook_event_name": "Notification",
            "session_id": self.session_id.as_deref().unwrap_or(""),
            "cwd": self.cwd.as_deref().unwrap_or(""),
            "transcript_path": "",
            "message": message,
            "title": title,
        });

        let output = self
            .invoke(HookEvent::Notification, notification_input, None, context)
            .await?;

        if output.decision.is_some()
            || output.system_message.is_some()
            || output.hook_specific_output.is_some()
        {
            outputs.push(output);
        }

        Ok(outputs)
    }

    /// Trigger `PermissionRequest` hook
    ///
    /// Called when a permission is requested for a tool.
    ///
    /// # Arguments
    /// * `tool_name` - Name of the tool requesting permission
    /// * `tool_input` - The tool's input parameters
    /// * `suggestions` - Optional permission suggestions
    ///
    /// # Errors
    ///
    /// Propagates errors from hook callback execution.
    pub async fn trigger_permission_request(
        &self,
        tool_name: &str,
        tool_input: serde_json::Value,
        suggestions: Option<Vec<serde_json::Value>>,
    ) -> Result<Vec<HookOutput>> {
        let mut outputs = Vec::new();
        let context = self.build_context();

        let permission_input = serde_json::json!({
            "hook_event_name": "PermissionRequest",
            "session_id": self.session_id.as_deref().unwrap_or(""),
            "cwd": self.cwd.as_deref().unwrap_or(""),
            "transcript_path": "",
            "tool_name": tool_name,
            "tool_input": tool_input,
            "permission_suggestions": suggestions,
        });

        let output = self
            .invoke(
                HookEvent::PermissionRequest,
                permission_input,
                Some(tool_name.to_string()),
                context,
            )
            .await?;

        if output.decision.is_some()
            || output.system_message.is_some()
            || output.hook_specific_output.is_some()
        {
            outputs.push(output);
        }

        Ok(outputs)
    }
}
