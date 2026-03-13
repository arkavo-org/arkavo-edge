//! Message sending, receiving, and buffering methods for `ClaudeSDKClient`

use crate::error::{ClaudeError, Result};
use crate::transport::Transport;
use crate::types::{HookEvent, Message, PermissionRequest, RequestId, SessionId};
use futures::Stream;
use tokio::sync::mpsc;

use super::ClaudeSDKClient;

impl ClaudeSDKClient {
    /// Send a message to Claude
    ///
    /// # Arguments
    /// * `content` - Message content to send
    ///
    /// # Errors
    /// Returns error if message cannot be sent
    pub async fn send_message(&mut self, content: impl Into<String>) -> Result<()> {
        // Validate session if bound
        self.validate_session()?;

        let content_str = content.into();

        // Trigger UserPromptSubmit hook before sending
        if let Some(ref manager) = self.hook_manager {
            let manager_guard = manager.lock().await;
            if let Err(e) = manager_guard.trigger_user_prompt_submit(&content_str).await {
                tracing::warn!(error = %e, "UserPromptSubmit hook error");
            }
        }

        // Send a user message in the format the CLI expects
        let message = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": content_str
            }
        });
        let message_json = format!("{}\n", serde_json::to_string(&message)?);

        let mut transport = self.transport.lock().await;
        transport.write(&message_json).await
    }

    // ========================================================================
    // Message Buffering
    // ========================================================================

    /// Queue a message to be sent after the current turn completes.
    ///
    /// The CLI only reads stdin between turns, not during streaming.
    /// Messages queued with this method are stored and can be sent
    /// automatically using `receive_buffered()` or manually with `send_queued()`.
    ///
    /// **Security**: Each queued message is associated with the current `session_id`.
    /// When sending, the SDK verifies the session hasn't changed, preventing
    /// messages from being accidentally sent to a different conversation context.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use anthropic_agent_sdk::{ClaudeSDKClient, ClaudeAgentOptions};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let options = ClaudeAgentOptions::default();
    /// let mut client = ClaudeSDKClient::new(options, None).await?;
    ///
    /// // Send first message
    /// client.send_message("What is Python?").await?;
    ///
    /// // Queue follow-up messages (will be sent after each Result)
    /// client.queue_message("What is TypeScript?");
    /// client.queue_message("Compare Rust to both.");
    ///
    /// // Process all messages with automatic queue handling
    /// while let Some(msg) = client.next_buffered().await {
    ///     // Handle messages...
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn queue_message(&self, content: impl Into<String>) {
        // Capture current session_id for security
        let session_id = self.get_session_id();
        if let Ok(mut buffer) = self.message_buffer.lock() {
            buffer.push_back((session_id, content.into()));
        }
    }

    /// Get the number of messages waiting in the queue.
    #[must_use]
    pub fn queued_count(&self) -> usize {
        self.message_buffer.lock().map(|b| b.len()).unwrap_or(0)
    }

    /// Check if there are messages waiting to be sent.
    #[must_use]
    pub fn has_queued(&self) -> bool {
        self.queued_count() > 0
    }

    /// Send the next queued message.
    ///
    /// Returns `Ok(true)` if a message was sent, `Ok(false)` if queue is empty.
    ///
    /// **Security**: Verifies the queued message's `session_id` matches the current
    /// session. If session changed, the message is discarded with a warning to
    /// prevent sending messages to an unintended conversation context.
    ///
    /// # Errors
    /// Returns error if message cannot be sent.
    pub async fn send_queued(&mut self) -> Result<bool> {
        let current_session = self.get_session_id();

        let next_entry = {
            self.message_buffer
                .lock()
                .ok()
                .and_then(|mut b| b.pop_front())
        };

        if let Some((queued_session, msg)) = next_entry {
            // Security check: verify session_id matches
            match (&queued_session, &current_session) {
                (Some(queued), Some(current)) if queued != current => {
                    // Session changed - discard message for safety
                    tracing::warn!(
                        queued_session = %queued,
                        current_session = %current,
                        "Discarding queued message: session_id changed"
                    );
                    // Clear remaining messages from old session
                    self.clear_queue();
                    return Ok(false);
                }
                _ => {
                    // Session matches or one is None (early in conversation)
                    self.send_message(msg).await?;
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    /// Get the next message, automatically sending queued messages after Results.
    ///
    /// This is the recommended way to handle multi-turn conversations with buffering.
    /// After receiving a Result message, any queued messages are automatically sent.
    /// Returns `None` when stream ends AND no more queued messages remain.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use anthropic_agent_sdk::{ClaudeSDKClient, ClaudeAgentOptions, Message};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let options = ClaudeAgentOptions::default();
    /// let mut client = ClaudeSDKClient::new(options, None).await?;
    ///
    /// client.send_message("What is Python?").await?;
    /// client.queue_message("What is TypeScript?");
    /// client.queue_message("Compare Rust to both.");
    ///
    /// while let Some(msg) = client.next_buffered().await {
    ///     match msg? {
    ///         Message::Assistant { message, .. } => {
    ///             println!("Claude: {:?}", message.content);
    ///         }
    ///         Message::Result { .. } => {
    ///             println!("Turn complete");
    ///             // next_buffered() automatically sends queued messages
    ///         }
    ///         _ => {}
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn next_buffered(&mut self) -> Option<Result<Message>> {
        match self.message_rx.recv().await {
            Some(result) => {
                // Check if this is a Result message
                if let Ok(Message::Result { .. }) = &result {
                    // After Result, try to send next queued message
                    if self.has_queued() {
                        let _ = self.send_queued().await;
                    }
                }
                Some(result)
            }
            None => None,
        }
    }

    /// Clear all queued messages.
    pub fn clear_queue(&self) {
        if let Ok(mut buffer) = self.message_buffer.lock() {
            buffer.clear();
        }
    }

    /// Send an interrupt signal
    ///
    /// **Note**: Interrupt functionality via control messages may not be fully supported
    /// in all Claude CLI versions. The method demonstrates the SDK's bidirectional
    /// capability and will send the control message without blocking, but the CLI
    /// may not process it. Check your CLI version for control message support.
    ///
    /// # Errors
    /// Returns error if interrupt cannot be sent
    pub async fn interrupt(&mut self) -> Result<()> {
        let protocol = self.protocol.lock().await;
        let request = protocol.create_interrupt_request();
        drop(protocol);

        self.control_tx
            .send(request)
            .map_err(|_| ClaudeError::transport("Control channel closed"))
    }

    /// Rewind files to a checkpoint
    ///
    /// Restores files to their state at the specified user message checkpoint.
    /// Requires `enable_file_checkpointing: true` in options.
    ///
    /// # Arguments
    /// * `user_message_uuid` - UUID from a User message's `uuid` field
    ///
    /// # Errors
    /// Returns error if the rewind request cannot be sent
    ///
    /// # Example
    /// ```rust,no_run
    /// # use anthropic_agent_sdk::{ClaudeSDKClient, ClaudeAgentOptions, Message};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let options = ClaudeAgentOptions::builder()
    ///     .enable_file_checkpointing(true)
    ///     .build();
    /// let mut client = ClaudeSDKClient::new(options, None).await?;
    ///
    /// // Capture checkpoint UUID from user messages
    /// let mut checkpoint_uuid: Option<String> = None;
    /// while let Some(msg) = client.next_message().await {
    ///     if let Message::User { uuid: Some(uuid), .. } = msg? {
    ///         checkpoint_uuid = Some(uuid);
    ///         break;
    ///     }
    /// }
    ///
    /// // Later, rewind to that checkpoint
    /// if let Some(uuid) = checkpoint_uuid {
    ///     client.rewind_files(&uuid).await?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn rewind_files(&mut self, user_message_uuid: &str) -> Result<()> {
        let protocol = self.protocol.lock().await;
        let request = protocol.create_rewind_files_request(user_message_uuid);
        drop(protocol);

        self.control_tx
            .send(request)
            .map_err(|_| ClaudeError::transport("Control channel closed"))
    }

    /// Get the next message from the stream
    ///
    /// Returns None when the stream ends
    pub async fn next_message(&mut self) -> Option<Result<Message>> {
        self.message_rx.recv().await
    }

    /// Take the hook event receiver
    ///
    /// This allows the caller to handle hook events independently
    pub fn take_hook_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<(String, HookEvent)>> {
        self.hook_rx.take()
    }

    /// Take the permission request receiver
    ///
    /// This allows the caller to handle permission requests independently
    pub fn take_permission_receiver(
        &mut self,
    ) -> Option<mpsc::UnboundedReceiver<(RequestId, PermissionRequest)>> {
        self.permission_rx.take()
    }

    /// Respond to a hook event
    ///
    /// # Arguments
    /// * `hook_id` - ID of the hook event being responded to
    /// * `response` - Hook response data
    ///
    /// # Errors
    /// Returns error if response cannot be sent
    pub async fn respond_to_hook(
        &mut self,
        hook_id: String,
        response: serde_json::Value,
    ) -> Result<()> {
        let protocol = self.protocol.lock().await;
        let request = protocol.create_hook_response(hook_id, response);
        drop(protocol);

        self.control_tx
            .send(request)
            .map_err(|_| ClaudeError::transport("Control channel closed"))
    }

    /// Respond to a permission request
    ///
    /// # Arguments
    /// * `request_id` - ID of the permission request being responded to
    /// * `result` - Permission result (Allow/Deny)
    ///
    /// # Errors
    /// Returns error if response cannot be sent
    pub async fn respond_to_permission(
        &mut self,
        request_id: RequestId,
        result: crate::types::PermissionResult,
    ) -> Result<()> {
        let protocol = self.protocol.lock().await;
        let request = protocol.create_permission_response(request_id, result);
        drop(protocol);

        self.control_tx
            .send(request)
            .map_err(|_| ClaudeError::transport("Control channel closed"))
    }

    /// Receive messages until a Result message is encountered.
    ///
    /// Returns a stream that yields messages and automatically terminates
    /// after yielding the final Result message. Convenient for single-query workflows.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use anthropic_agent_sdk::{ClaudeSDKClient, ClaudeAgentOptions, Message};
    /// # use futures::StreamExt;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let options = ClaudeAgentOptions::default();
    /// # let mut client = ClaudeSDKClient::new(options, None).await?;
    /// client.send_message("Hello").await?;
    ///
    /// let mut messages = Box::pin(client.receive_response());
    /// while let Some(msg) = messages.next().await {
    ///     match msg? {
    ///         Message::Assistant { message, .. } => println!("{:?}", message),
    ///         Message::Result { .. } => println!("Done!"),
    ///         _ => {}
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[must_use = "receive_response returns a stream that must be consumed to receive messages"]
    pub fn receive_response(&mut self) -> impl Stream<Item = Result<Message>> + '_ {
        async_stream::stream! {
            while let Some(result) = self.message_rx.recv().await {
                let is_result = matches!(&result, Ok(Message::Result { .. }));
                yield result;
                if is_result {
                    break;
                }
            }
        }
    }

    /// Check if the client is currently connected.
    ///
    /// Returns `true` if the transport is connected and ready.
    pub async fn is_connected(&self) -> bool {
        let transport = self.transport.lock().await;
        transport.is_ready()
    }

    /// Get the current session ID if available.
    ///
    /// The session ID is captured from Result messages automatically.
    /// Returns `None` if no session has been established yet.
    #[must_use]
    pub fn get_session_id(&self) -> Option<SessionId> {
        self.session_id.lock().ok()?.clone()
    }
}
