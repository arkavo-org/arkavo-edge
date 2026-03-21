//! `ClaudeSDKClient` for bidirectional communication
//!
//! This module provides the main client for interactive, stateful conversations
//! with Claude Code, including support for:
//! - Bidirectional messaging (no lock contention)
//! - Interrupts and control flow
//! - Hook and permission callbacks
//! - Conversation state management
//!
//! # Architecture
//!
//! The client uses a lock-free architecture for reading and writing:
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                   ClaudeSDKClient                        │
//! │                                                          │
//! │  ┌──────────────────┐        ┌──────────────────┐      │
//! │  │  Message Reader  │        │  Control Writer  │      │
//! │  │  Background Task │        │  Background Task │      │
//! │  │                  │        │                  │      │
//! │  │ • Gets receiver  │        │ • Locks per-write│      │
//! │  │   once           │        │ • No blocking    │      │
//! │  │ • No lock held   │        │                  │      │
//! │  │   while reading  │        │                  │      │
//! │  └────────┬─────────┘        └────────┬─────────┘      │
//! │           │                           │                 │
//! │           │    ┌──────────────┐      │                 │
//! │           └───→│  Transport   │←─────┘                 │
//! │                │  (Arc<Mutex>)│                         │
//! │                └──────────────┘                         │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! **Key Design Points:**
//! - Transport returns an owned `UnboundedReceiver` (no lifetime issues)
//! - Reader task gets receiver once, then releases transport lock
//! - Writer task locks transport briefly for each write operation
//! - No contention: reader never blocks writer, writer never blocks reader
//!
//! # Example: Basic Usage
//!
//! ```no_run
//! use anthropic_agent_sdk::{ClaudeSDKClient, ClaudeAgentOptions, Message};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let options = ClaudeAgentOptions::default();
//! let mut client = ClaudeSDKClient::new(options, None).await?;
//!
//! // Send a message
//! client.send_message("Hello, Claude!").await?;
//!
//! // Read responses
//! while let Some(message) = client.next_message().await {
//!     match message? {
//!         Message::Assistant { message, .. } => {
//!             println!("Response: {:?}", message.content);
//!         }
//!         Message::Result { .. } => break,
//!         _ => {}
//!     }
//! }
//!
//! client.close().await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Example: Concurrent Operations
//!
//! ```no_run
//! use anthropic_agent_sdk::{ClaudeSDKClient, ClaudeAgentOptions};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let options = ClaudeAgentOptions::default();
//! let mut client = ClaudeSDKClient::new(options, None).await?;
//!
//! // Send first message
//! client.send_message("First question").await?;
//!
//! // Can send another message while reading responses
//! // No blocking due to lock-free architecture
//! tokio::spawn(async move {
//!     tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
//!     client.send_message("Second question").await
//! });
//!
//! # Ok(())
//! # }
//! ```
//!
//! # Example: Interrupt
//!
//! ```no_run
//! use anthropic_agent_sdk::{ClaudeSDKClient, ClaudeAgentOptions};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let options = ClaudeAgentOptions::default();
//! let mut client = ClaudeSDKClient::new(options, None).await?;
//!
//! client.send_message("Write a long essay").await?;
//!
//! // After some time, interrupt the response
//! tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
//! client.interrupt().await?;
//!
//! # Ok(())
//! # }
//! ```
//!
//! # Example: Hooks and Permissions
//!
//! ```no_run
//! use anthropic_agent_sdk::{ClaudeSDKClient, ClaudeAgentOptions};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let options = ClaudeAgentOptions::default();
//! let mut client = ClaudeSDKClient::new(options, None).await?;
//!
//! // Take receivers to handle hooks and permissions
//! let mut hook_rx = client.take_hook_receiver().unwrap();
//! let mut perm_rx = client.take_permission_receiver().unwrap();
//!
//! // Handle hook events
//! tokio::spawn(async move {
//!     while let Some((hook_id, event)) = hook_rx.recv().await {
//!         println!("Hook: {} {:?}", hook_id, event);
//!         // Respond to hook...
//!     }
//! });
//!
//! // Handle permission requests
//! tokio::spawn(async move {
//!     while let Some((req_id, request)) = perm_rx.recv().await {
//!         println!("Permission: {:?}", request);
//!         // Respond to permission...
//!     }
//! });
//!
//! # Ok(())
//! # }
//! ```

mod messaging;
mod session;
mod tasks;

use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::control::{ControlRequest, ProtocolHandler};
use crate::error::Result;
use crate::hooks::HookManager;
use crate::permissions::PermissionManager;
use crate::transport::{PromptInput, SubprocessTransport, Transport};
use crate::types::{
    ClaudeAgentOptions, HookEvent, PermissionRequest, RequestId, SessionId, SessionInfo,
};

/// A buffered message with its associated session ID for security validation
type BufferedMessage = (Option<SessionId>, String);

/// Thread-safe queue for buffering messages during streaming
type MessageBuffer = Arc<std::sync::Mutex<VecDeque<BufferedMessage>>>;

/// Context for the message reader background task
pub(crate) struct MessageReaderContext {
    pub(crate) transport: Arc<Mutex<SubprocessTransport>>,
    pub(crate) protocol: Arc<Mutex<ProtocolHandler>>,
    pub(crate) message_tx: mpsc::UnboundedSender<Result<Message>>,
    pub(crate) session_id: Arc<std::sync::Mutex<Option<SessionId>>>,
    pub(crate) session_info: Arc<std::sync::Mutex<Option<SessionInfo>>>,
    pub(crate) bound_session_id: Arc<std::sync::Mutex<Option<SessionId>>>,
    pub(crate) hook_manager: Option<Arc<Mutex<HookManager>>>,
    pub(crate) is_resume: bool,
}

use crate::types::Message;

/// Client for bidirectional communication with Claude Code
///
/// `ClaudeSDKClient` provides interactive, stateful conversations with
/// support for interrupts, hooks, and permission callbacks.
///
/// # Examples
///
/// ```no_run
/// use anthropic_agent_sdk::{ClaudeSDKClient, ClaudeAgentOptions};
/// use futures::StreamExt;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let options = ClaudeAgentOptions::default();
///     let mut client = ClaudeSDKClient::new(options, None).await?;
///
///     client.send_message("Hello, Claude!").await?;
///
///     while let Some(message) = client.next_message().await {
///         println!("{:?}", message?);
///     }
///
///     Ok(())
/// }
/// ```
pub struct ClaudeSDKClient {
    /// Transport layer
    transport: Arc<Mutex<SubprocessTransport>>,
    /// Control protocol handler
    protocol: Arc<Mutex<ProtocolHandler>>,
    /// Message stream receiver
    message_rx: mpsc::UnboundedReceiver<Result<Message>>,
    /// Control message sender
    control_tx: mpsc::UnboundedSender<ControlRequest>,
    /// Hook event receiver (if not using automatic handler)
    hook_rx: Option<mpsc::UnboundedReceiver<(String, HookEvent)>>,
    /// Permission request receiver (if not using automatic handler)
    permission_rx: Option<mpsc::UnboundedReceiver<(RequestId, PermissionRequest)>>,
    /// Hook manager for automatic hook handling
    hook_manager: Option<Arc<Mutex<HookManager>>>,
    /// Permission manager kept alive for background permission handler task
    _permission_manager: Option<Arc<Mutex<PermissionManager>>>,
    /// Captured session ID from messages
    session_id: Arc<std::sync::Mutex<Option<SessionId>>>,
    /// Session info from init message (model, tools, MCP servers)
    session_info: Arc<std::sync::Mutex<Option<SessionInfo>>>,
    /// Cancellation token for aborting operations (like `AbortController` in JS)
    cancellation_token: CancellationToken,
    /// Runtime model override
    runtime_model: Arc<std::sync::Mutex<Option<String>>>,
    /// Runtime permission mode override
    runtime_permission_mode: Arc<std::sync::Mutex<Option<crate::types::PermissionMode>>>,
    /// Runtime max thinking tokens override
    runtime_max_thinking_tokens: Arc<std::sync::Mutex<Option<u32>>>,
    /// Message buffer for queuing messages during streaming
    message_buffer: MessageBuffer,
    /// Bound session ID - if set, all sends validate against this
    bound_session_id: Arc<std::sync::Mutex<Option<SessionId>>>,
}

impl ClaudeSDKClient {
    /// Create a new `ClaudeSDKClient`
    ///
    /// # Arguments
    /// * `options` - Configuration options
    /// * `cli_path` - Optional path to Claude Code CLI
    ///
    /// # Errors
    /// Returns error if CLI cannot be found or connection fails
    pub async fn new(
        options: ClaudeAgentOptions,
        cli_path: Option<std::path::PathBuf>,
    ) -> Result<Self> {
        // Create cancellation token (like AbortController in JavaScript)
        let cancellation_token = CancellationToken::new();

        // Initialize hook manager if hooks are configured
        let (hook_manager, hook_rx) = if let Some(ref hooks_config) = options.hooks {
            let mut manager = HookManager::from_hooks_config(hooks_config.clone());
            // Set cancellation token so hooks can check for abort
            manager.set_cancellation_token(cancellation_token.child_token());
            (Some(Arc::new(Mutex::new(manager))), None)
        } else {
            (None, Some(mpsc::unbounded_channel().1))
        };

        // Initialize permission manager if callback is configured
        let (permission_manager, permission_rx) = if options.can_use_tool.is_some() {
            let mut manager = PermissionManager::new();
            if let Some(callback) = options.can_use_tool.clone() {
                manager.set_callback(callback);
            }
            manager.set_allowed_tools(Some(options.allowed_tools.clone()));
            manager.set_disallowed_tools(options.disallowed_tools.clone());
            (Some(Arc::new(Mutex::new(manager))), None)
        } else {
            (None, Some(mpsc::unbounded_channel().1))
        };

        // Check if this is a resume session (for SessionStart hook)
        let is_resume = options.resume.is_some();

        // Create transport with streaming mode and pass child cancellation token
        let prompt_input = PromptInput::Stream;
        let mut transport = SubprocessTransport::with_cancellation_token(
            prompt_input,
            options,
            cli_path,
            Some(cancellation_token.child_token()),
        )?;

        // Connect transport
        transport.connect().await?;

        // Create protocol handler
        let mut protocol = ProtocolHandler::new();

        // Set up channels
        let (hook_tx, hook_rx_internal) = mpsc::unbounded_channel();
        let (permission_tx, permission_rx_internal) = mpsc::unbounded_channel();
        protocol.set_hook_channel(hook_tx);
        protocol.set_permission_channel(permission_tx);

        let (message_tx, message_rx) = mpsc::unbounded_channel();
        let (control_tx, control_rx) = mpsc::unbounded_channel();

        // Note: Claude CLI doesn't use a separate control protocol initialization.
        // The stream-json mode expects user messages to be sent directly.
        // Mark protocol as initialized immediately.
        protocol.set_initialized(true);

        let transport = Arc::new(Mutex::new(transport));
        let protocol = Arc::new(Mutex::new(protocol));
        let session_id = Arc::new(std::sync::Mutex::new(None));
        let session_info = Arc::new(std::sync::Mutex::new(None));
        let bound_session_id = Arc::new(std::sync::Mutex::new(None));

        // Spawn message reader task
        let reader_ctx = MessageReaderContext {
            transport: transport.clone(),
            protocol: protocol.clone(),
            message_tx,
            session_id: session_id.clone(),
            session_info: session_info.clone(),
            bound_session_id: bound_session_id.clone(),
            hook_manager: hook_manager.clone(),
            is_resume,
        };
        tokio::spawn(async move {
            Self::message_reader_task(reader_ctx).await;
        });

        // Spawn control message writer task
        let transport_clone = transport.clone();
        let protocol_clone = protocol.clone();
        tokio::spawn(async move {
            Self::control_writer_task(transport_clone, protocol_clone, control_rx).await;
        });

        // Spawn hook handler task if hook manager is configured
        if let Some(ref manager) = hook_manager {
            let manager_clone = manager.clone();
            let protocol_clone = protocol.clone();
            let control_tx_clone = control_tx.clone();
            tokio::spawn(async move {
                Self::hook_handler_task(
                    manager_clone,
                    protocol_clone,
                    control_tx_clone,
                    hook_rx_internal,
                )
                .await;
            });
        }

        // Spawn permission handler task if permission manager is configured
        if let Some(ref manager) = permission_manager {
            let manager_clone = manager.clone();
            let protocol_clone = protocol.clone();
            let control_tx_clone = control_tx.clone();
            tokio::spawn(async move {
                Self::permission_handler_task(
                    manager_clone,
                    protocol_clone,
                    control_tx_clone,
                    permission_rx_internal,
                )
                .await;
            });
        }

        Ok(Self {
            transport,
            protocol,
            message_rx,
            control_tx,
            hook_rx,
            permission_rx,
            hook_manager,
            _permission_manager: permission_manager,
            session_id,
            session_info,
            cancellation_token,
            runtime_model: Arc::new(std::sync::Mutex::new(None)),
            runtime_permission_mode: Arc::new(std::sync::Mutex::new(None)),
            runtime_max_thinking_tokens: Arc::new(std::sync::Mutex::new(None)),
            message_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
            bound_session_id,
        })
    }
}

impl Drop for ClaudeSDKClient {
    fn drop(&mut self) {
        // Channel senders will be dropped, causing background tasks to exit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("CASDK-002")]
    #[tokio::test]
    async fn test_client_creation_without_cli() {
        // When CLI is not found, client creation should fail with a clear error
        let options = ClaudeAgentOptions::default();
        let result = ClaudeSDKClient::new(options, Some("/nonexistent/claude".into())).await;
        assert!(result.is_err(), "Should fail when CLI path doesn't exist");
    }

    #[spec("CASDK-002")]
    #[tokio::test]
    async fn test_session_id_initially_none() {
        let options = ClaudeAgentOptions::default();
        if let Ok(client) = ClaudeSDKClient::new(options, None).await {
            // Session ID should be None before any Result message is received
            assert!(client.get_session_id().is_none());
        }
    }

    #[spec("CASDK-002")]
    #[tokio::test]
    async fn test_is_connected() {
        let options = ClaudeAgentOptions::default();
        if let Ok(client) = ClaudeSDKClient::new(options, None).await {
            // Should be connected after successful initialization
            assert!(client.is_connected().await);
        }
    }
}
