//! Session binding, cancellation, and close for `ClaudeSDKClient`

mod introspection;

use crate::error::{ClaudeError, Result};
use crate::transport::Transport;
use crate::types::SessionId;

use super::ClaudeSDKClient;

impl ClaudeSDKClient {
    // ========================================================================
    // Session Binding
    // ========================================================================

    /// Bind this client to a specific session ID.
    ///
    /// Once bound, all `send_message()` calls validate that the current
    /// session matches the bound session. If a mismatch is detected,
    /// `send_message()` returns `ClaudeError::SessionMismatch`.
    ///
    /// This provides defense in depth to prevent messages from being
    /// accidentally sent to a different conversation context.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use anthropic_agent_sdk::{ClaudeSDKClient, ClaudeAgentOptions, Message};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let options = ClaudeAgentOptions::default();
    /// let mut client = ClaudeSDKClient::new(options, None).await?;
    ///
    /// client.send_message("Hello").await?;
    ///
    /// while let Some(msg) = client.next_message().await {
    ///     if let Message::Result { session_id, .. } = msg? {
    ///         // Bind to this session - all future sends will validate
    ///         client.bind_session(session_id);
    ///         break;
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn bind_session(&self, session_id: SessionId) {
        if let Ok(mut guard) = self.bound_session_id.lock() {
            *guard = Some(session_id);
        }
    }

    /// Get the bound session ID, if any.
    ///
    /// Returns `None` if no session is bound (binding disabled).
    #[must_use]
    pub fn bound_session(&self) -> Option<SessionId> {
        self.bound_session_id.lock().ok()?.clone()
    }

    /// Clear session binding, allowing messages to any session.
    ///
    /// After calling this, `send_message()` will no longer validate
    /// session IDs before sending.
    pub fn unbind_session(&self) {
        if let Ok(mut guard) = self.bound_session_id.lock() {
            *guard = None;
        }
    }

    /// Validate that current session matches bound session.
    ///
    /// Returns `Ok(())` if:
    /// - No session is bound (binding disabled)
    /// - Current session matches bound session
    /// - Either session is None (early in conversation)
    ///
    /// This is called automatically by `send_message()` if a session is bound.
    /// You can also call it manually for explicit validation.
    ///
    /// # Errors
    ///
    /// Returns `SessionMismatch` if bound session differs from current session.
    pub fn validate_session(&self) -> Result<()> {
        let bound = self.bound_session();
        let current = self.get_session_id();

        match (&bound, &current) {
            (Some(b), Some(c)) if b != c => {
                Err(ClaudeError::session_mismatch(b.to_string(), c.to_string()))
            }
            _ => Ok(()),
        }
    }

    // ========================================================================
    // Cancellation
    // ========================================================================

    /// Get a child cancellation token for this client.
    ///
    /// This is analogous to JavaScript's `AbortController.signal`. Callers can
    /// use the returned token to:
    /// - Check if cancellation was requested: `token.is_cancelled()`
    /// - Wait for cancellation: `token.cancelled().await`
    /// - Use with `tokio::select!` to race cancellation against other futures
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use anthropic_agent_sdk::{ClaudeSDKClient, ClaudeAgentOptions};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let options = ClaudeAgentOptions::default();
    /// let mut client = ClaudeSDKClient::new(options, None).await?;
    ///
    /// let cancel_token = client.cancellation_token();
    ///
    /// // Use in a spawned task to respect cancellation
    /// let token = cancel_token.clone();
    /// tokio::spawn(async move {
    ///     tokio::select! {
    ///         _ = token.cancelled() => {
    ///             println!("Operation cancelled");
    ///         }
    ///         _ = async { /* long operation */ } => {
    ///             println!("Operation completed");
    ///         }
    ///     }
    /// });
    ///
    /// // Later, cancel all operations
    /// client.cancel();
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn cancellation_token(&self) -> tokio_util::sync::CancellationToken {
        self.cancellation_token.child_token()
    }

    /// Cancel all ongoing operations.
    ///
    /// This is analogous to JavaScript's `AbortController.abort()`. Calling this
    /// method will:
    /// - Cancel the message reader in the transport
    /// - Signal any operations using a child cancellation token to stop
    ///
    /// Unlike `close()`, this does not immediately close the client - it only
    /// signals cancellation. Use `close()` after `cancel()` for full cleanup.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use anthropic_agent_sdk::{ClaudeSDKClient, ClaudeAgentOptions};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let options = ClaudeAgentOptions::default();
    /// let mut client = ClaudeSDKClient::new(options, None).await?;
    ///
    /// client.send_message("Write a long essay").await?;
    ///
    /// // After some condition, cancel operations
    /// client.cancel();
    ///
    /// // Then close the client
    /// client.close().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn cancel(&self) {
        self.cancellation_token.cancel();
    }

    /// Check if cancellation has been requested.
    ///
    /// Returns `true` if `cancel()` has been called on this client.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancellation_token.is_cancelled()
    }

    // ========================================================================
    // Close
    // ========================================================================

    /// Close the client and clean up resources
    ///
    /// # Errors
    /// Returns error if cleanup fails
    pub async fn close(&mut self) -> Result<()> {
        // Trigger SessionEnd hook before closing
        if let Some(ref manager) = self.hook_manager {
            let manager_guard = manager.lock().await;
            if let Err(e) = manager_guard.trigger_session_end("other").await {
                tracing::warn!(error = %e, "SessionEnd hook error");
            }
        }

        let mut transport = self.transport.lock().await;
        transport.close().await
    }
}
