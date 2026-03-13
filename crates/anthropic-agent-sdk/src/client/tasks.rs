//! Background task methods for `ClaudeSDKClient`

use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

use crate::control::{ControlMessage, ControlRequest, ProtocolHandler};
use crate::hooks::HookManager;
use crate::message::parse_message;
use crate::permissions::PermissionManager;
use crate::transport::{SubprocessTransport, Transport};
use crate::types::{HookEvent, Message, PermissionRequest, RequestId, SessionId, SessionInfo};

use super::{ClaudeSDKClient, MessageReaderContext};

impl ClaudeSDKClient {
    /// Message reader task - reads from transport and processes messages
    ///
    /// If `hook_manager` is provided, automatically calls `process_message()` on each
    /// message to trigger registered hooks (`SubagentStart`, `SubagentStop`, etc.)
    #[allow(clippy::too_many_lines)]
    pub(crate) async fn message_reader_task(ctx: MessageReaderContext) {
        let MessageReaderContext {
            transport,
            protocol,
            message_tx,
            session_id,
            session_info,
            bound_session_id,
            hook_manager,
            is_resume,
        } = ctx;
        // Get the message receiver from the transport without holding the lock
        let mut msg_stream = {
            let mut transport_guard = transport.lock().await;
            transport_guard.read_messages()
        };

        while let Some(result) = msg_stream.recv().await {
            match result {
                Ok(value) => {
                    // Try to parse as control message first
                    let protocol_guard = protocol.lock().await;
                    let value_str = serde_json::to_string(&value).unwrap_or_default();
                    if let Ok(control_msg) = protocol_guard.deserialize_message(&value_str) {
                        tracing::trace!("Parsed as control message, consuming internally");

                        match control_msg {
                            ControlMessage::InitResponse(init_response) => {
                                if let Err(e) = protocol_guard.handle_init_response(&init_response)
                                {
                                    let _ = message_tx.send(Err(e));
                                    break;
                                }
                            }
                            ControlMessage::Response(response) => {
                                if let Err(e) = protocol_guard.handle_response(response).await {
                                    let _ = message_tx.send(Err(e));
                                }
                            }
                            ControlMessage::Request(_) | ControlMessage::Init(_) => {
                                // Ignore requests and init in client mode
                            }
                        }
                        drop(protocol_guard);
                        continue;
                    }
                    drop(protocol_guard);

                    // Check for control_response (ack from CLI for control_request)
                    // These are internal protocol messages, not user-facing
                    if let Some(msg_type) = value.get("type").and_then(|v| v.as_str())
                        && msg_type == "control_response"
                    {
                        tracing::debug!(
                            request_id = %value.get("response")
                                .and_then(|r| r.get("request_id"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown"),
                            "Received control_response (interrupt/setModel/etc ack)"
                        );
                        continue;
                    }

                    // Otherwise parse as regular message
                    tracing::trace!(
                        preview = %serde_json::to_string(&value).unwrap_or_default().chars().take(100).collect::<String>(),
                        "Parsing as Message"
                    );

                    match parse_message(value) {
                        Ok(msg) => {
                            tracing::trace!("Parsed message successfully, sending to channel");

                            // Capture session_id from Result messages
                            if let Message::Result {
                                session_id: ref sid,
                                ..
                            } = msg
                            {
                                if let Ok(mut session_guard) = session_id.lock() {
                                    *session_guard = Some(sid.clone());
                                }
                                // Auto-bind to session on first Result (secure by default)
                                if let Ok(mut bound_guard) = bound_session_id.lock()
                                    && bound_guard.is_none()
                                {
                                    *bound_guard = Some(sid.clone());
                                }
                            }

                            // Capture session info from System init message and pass to HookManager
                            if let Message::System {
                                ref subtype,
                                ref data,
                            } = msg
                                && subtype == "init"
                            {
                                let init_session_id = data
                                    .get("session_id")
                                    .and_then(|v| v.as_str())
                                    .map(std::string::ToString::to_string);
                                let init_cwd = data
                                    .get("cwd")
                                    .and_then(|v| v.as_str())
                                    .map(std::string::ToString::to_string);

                                // Update session_id storage
                                if let Some(ref sid) = init_session_id
                                    && let Ok(mut session_guard) = session_id.lock()
                                {
                                    *session_guard = Some(SessionId::from(sid.clone()));
                                }

                                // Populate session_info from init data
                                if let Ok(mut info_guard) = session_info.lock() {
                                    *info_guard = Some(SessionInfo::from_init_data(data));
                                }

                                // Update HookManager with session context and trigger SessionStart
                                if let Some(ref manager) = hook_manager
                                    && let Some(sid) = init_session_id
                                {
                                    // Determine session start source
                                    let source = if is_resume { "resume" } else { "startup" };

                                    {
                                        let mut manager_guard = manager.lock().await;
                                        manager_guard.set_session_context(sid, init_cwd);

                                        // Trigger SessionStart hook
                                        if let Err(e) =
                                            manager_guard.trigger_session_start(source).await
                                        {
                                            tracing::warn!(error = %e, "SessionStart hook error");
                                        }
                                    }

                                    tracing::debug!(source = source, "Triggered SessionStart hook");
                                }
                            }

                            // Process message through hook manager if configured
                            // This triggers SubagentStart, SubagentStop, PreToolUse, PostToolUse hooks
                            if let Some(ref manager) = hook_manager {
                                let mut manager_guard = manager.lock().await;
                                match manager_guard.process_message(&msg).await {
                                    Ok(outputs) => {
                                        if !outputs.is_empty() {
                                            tracing::debug!(
                                                count = outputs.len(),
                                                "Hook outputs from message processing"
                                            );
                                        }
                                        // Hook outputs are handled internally by callbacks
                                        // Future: could send outputs to a channel for external handling
                                    }
                                    Err(e) => {
                                        tracing::warn!(error = %e, "Hook processing error");
                                    }
                                }
                            }

                            if message_tx.send(Ok(msg)).is_err() {
                                tracing::warn!(
                                    "Failed to send message to channel - receiver dropped"
                                );
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::debug!(error = ?e, "Failed to parse message");
                            let _ = message_tx.send(Err(e));
                        }
                    }
                }
                Err(e) => {
                    let _ = message_tx.send(Err(e));
                    break;
                }
            }
        }
    }

    /// Control message writer task - writes control requests to transport
    ///
    /// Sends control requests using the Claude CLI streaming protocol format:
    /// ```json
    /// {"type": "control_request", "request_id": "...", "request": {"subtype": "..."}}
    /// ```
    pub(crate) async fn control_writer_task(
        transport: Arc<Mutex<SubprocessTransport>>,
        _protocol: Arc<Mutex<ProtocolHandler>>,
        mut control_rx: mpsc::UnboundedReceiver<ControlRequest>,
    ) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

        while let Some(request) = control_rx.recv().await {
            // Generate unique request ID matching TypeScript SDK format
            let request_id = format!(
                "req_{}_{:x}",
                REQUEST_COUNTER.fetch_add(1, Ordering::SeqCst),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            );

            // Build the inner request object based on control type
            let inner_request = match request {
                ControlRequest::Interrupt { .. } => {
                    serde_json::json!({"subtype": "interrupt"})
                }
                ControlRequest::SendMessage { content, .. } => {
                    // User messages should go through send_message, not control channel
                    // But handle it anyway for robustness
                    serde_json::json!({
                        "type": "user",
                        "message": {
                            "role": "user",
                            "content": content
                        }
                    })
                }
                ControlRequest::SetModel { model, .. } => {
                    serde_json::json!({
                        "subtype": "set_model",
                        "model": model
                    })
                }
                ControlRequest::SetPermissionMode { mode, .. } => {
                    serde_json::json!({
                        "subtype": "set_permission_mode",
                        "mode": mode
                    })
                }
                ControlRequest::SetMaxThinkingTokens {
                    max_thinking_tokens,
                    ..
                } => {
                    serde_json::json!({
                        "subtype": "set_max_thinking_tokens",
                        "max_thinking_tokens": max_thinking_tokens
                    })
                }
                ControlRequest::RewindFiles {
                    user_message_uuid, ..
                } => {
                    // Try snake_case format (CLI may use different format than TS SDK)
                    serde_json::json!({
                        "subtype": "rewind_files",
                        "user_message_uuid": user_message_uuid
                    })
                }
                _ => {
                    // Other control types not yet supported in stream-json mode
                    tracing::debug!(request = ?request, "Skipping unsupported control request");
                    continue;
                }
            };

            // Wrap in control_request envelope (matches TypeScript SDK protocol)
            let control_json = serde_json::json!({
                "type": "control_request",
                "request_id": request_id,
                "request": inner_request
            });

            if let Ok(json_str) = serde_json::to_string(&control_json) {
                tracing::debug!(json = %json_str, "Sending control request to CLI");
                let message_line = format!("{json_str}\n");
                let mut transport_guard = transport.lock().await;
                if transport_guard.write(&message_line).await.is_err() {
                    tracing::error!("Failed to write control request to CLI");
                    break;
                }
            } else {
                tracing::error!("Failed to serialize control request");
                break;
            }
        }
    }

    /// Hook handler task - automatically processes hook events
    pub(crate) async fn hook_handler_task(
        manager: Arc<Mutex<HookManager>>,
        protocol: Arc<Mutex<ProtocolHandler>>,
        control_tx: mpsc::UnboundedSender<ControlRequest>,
        mut hook_rx: mpsc::UnboundedReceiver<(String, HookEvent)>,
    ) {
        while let Some((hook_id, event)) = hook_rx.recv().await {
            let manager_guard = manager.lock().await;
            let context = manager_guard.build_context();

            match manager_guard
                .invoke(event, serde_json::json!({}), None, context)
                .await
            {
                Ok(output) => {
                    drop(manager_guard);

                    let protocol_guard = protocol.lock().await;
                    let response = serde_json::to_value(&output).unwrap_or_default();
                    let request = protocol_guard.create_hook_response(hook_id, response);
                    drop(protocol_guard);

                    let _ = control_tx.send(request);
                    tracing::debug!(event = ?event, "Hook processed and response sent");
                }
                Err(e) => {
                    tracing::error!(error = %e, "Hook processing error");
                }
            }
        }
    }

    /// Permission handler task - automatically processes permission requests
    pub(crate) async fn permission_handler_task(
        manager: Arc<Mutex<PermissionManager>>,
        protocol: Arc<Mutex<ProtocolHandler>>,
        control_tx: mpsc::UnboundedSender<ControlRequest>,
        mut permission_rx: mpsc::UnboundedReceiver<(RequestId, PermissionRequest)>,
    ) {
        while let Some((request_id, request)) = permission_rx.recv().await {
            let manager_guard = manager.lock().await;

            match manager_guard
                .can_use_tool(
                    request.tool_name.clone(),
                    request.tool_input.clone(),
                    request.context.clone(),
                )
                .await
            {
                Ok(result) => {
                    drop(manager_guard);

                    let protocol_guard = protocol.lock().await;
                    let request = protocol_guard
                        .create_permission_response(request_id.clone(), result.clone());
                    drop(protocol_guard);

                    let _ = control_tx.send(request);
                    tracing::debug!(request_id = %request_id.as_str(), result = ?result, "Permission processed and response sent");
                }
                Err(e) => {
                    tracing::error!(error = %e, "Permission processing error");
                }
            }
        }
    }
}
