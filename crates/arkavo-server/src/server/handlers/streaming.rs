use arkavo_llm::{DeltaStream, DeltaType, LlmClientAdapter, StreamId, StreamLlmModel};
use arkavo_protocol::chat_session::ChatSessionManager;
use arkavo_protocol::metrics::{MetricsCollector, RpcTimer};
use arkavo_protocol::rate_limit::RateLimiter;
use arkavo_protocol::types::{ChatRequest, MessageDelta, MessageDeltaContent};
use futures::StreamExt;
use jsonrpsee::{PendingSubscriptionSink, SubscriptionMessage, core::SubscriptionResult};
use std::sync::Arc;
use tracing::{error, info, warn};

use super::super::config_helpers::AgentMetadata;

#[allow(clippy::used_underscore_binding)]
pub async fn handle_message_stream(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    sink: PendingSubscriptionSink,
    _task_id: String,
) -> SubscriptionResult {
    let timer = RpcTimer::new("message/stream".to_string(), metrics.clone());

    if let Err(_e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Ok(());
    }

    let _sink = match sink.accept().await {
        Ok(sink) => sink,
        Err(_) => {
            timer.error();
            return Ok(());
        }
    };

    #[cfg(feature = "stub_handlers")]
    {
        tokio::spawn(async move {
            let delta = MessageDelta {
                session_id: _task_id.clone(),
                message_id: uuid::Uuid::new_v4().to_string(),
                sequence: 0,
                delta: MessageDeltaContent::Text {
                    text: "Processing task...".to_string(),
                },
                timestamp: chrono::Utc::now(),
            };

            if let Ok(msg) = SubscriptionMessage::from_json(&delta) {
                let _ = _sink.send(msg).await;
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

            let delta = MessageDelta {
                session_id: _task_id,
                message_id: uuid::Uuid::new_v4().to_string(),
                sequence: 1,
                delta: MessageDeltaContent::StreamEnd {
                    reason: StreamEndReason::Complete,
                },
                timestamp: chrono::Utc::now(),
            };

            if let Ok(msg) = SubscriptionMessage::from_json(&delta) {
                let _ = _sink.send(msg).await;
            }
        });
    }

    timer.success();
    Ok(())
}

pub async fn handle_chat_stream(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    chat_sessions: &Arc<ChatSessionManager>,
    sink: PendingSubscriptionSink,
    session_id: String,
) -> SubscriptionResult {
    info!(session.id = %session_id, "chat_stream subscription requested");
    let timer = RpcTimer::new("chat_stream".to_string(), metrics.clone());

    if let Err(_e) = rate_limiter.check_rate_limit() {
        warn!(session.id = %session_id, "Rate limit exceeded for chat_stream");
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Ok(());
    }

    info!(session.id = %session_id, "Accepting chat_stream subscription");
    let sink = match sink.accept().await {
        Ok(sink) => {
            info!(session.id = %session_id, "Subscription accepted successfully");
            sink
        }
        Err(e) => {
            error!(session.id = %session_id, error = ?e, "Failed to accept subscription");
            timer.error();
            return Ok(());
        }
    };

    if let Some(mut delta_rx) = chat_sessions.get_delta_stream(&session_id).await {
        info!(session.id = %session_id, "Got delta stream, spawning forwarder task");

        tokio::spawn(async move {
            info!(session.id = %session_id, "Delta forwarder task started");
            let mut delta_count = 0;
            while let Some(delta) = delta_rx.recv().await {
                delta_count += 1;
                info!(session.id = %session_id, delta_count, "Forwarding delta to client");
                if let Ok(msg) = SubscriptionMessage::from_json(&delta)
                    && sink.send(msg.clone()).await.is_err()
                {
                    warn!(session.id = %session_id, "Client disconnected, stopping delta forwarding");
                    break;
                }
            }
            info!(session.id = %session_id, total_deltas = delta_count, "Delta forwarder task ended");
        });

        timer.success();
        Ok(())
    } else {
        error!(session.id = %session_id, "Session not found for chat_stream subscription");
        timer.error();
        Ok(())
    }
}

pub async fn handle_chat_subscribe(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    llm_adapter: Option<Arc<LlmClientAdapter>>,
    agent_metadata: AgentMetadata,
    sink: PendingSubscriptionSink,
    request: ChatRequest,
) -> SubscriptionResult {
    let timer = RpcTimer::new("chat_subscribe".to_string(), metrics.clone());

    if let Err(_e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Ok(());
    }

    let sink = match sink.accept().await {
        Ok(sink) => sink,
        Err(_) => {
            timer.error();
            return Ok(());
        }
    };

    let message_id = uuid::Uuid::new_v4().to_string();
    let trace_id = uuid::Uuid::new_v4().to_string();

    tokio::spawn(async move {
        if let Some(adapter) = llm_adapter {
            let chat_request = arkavo_llm::ChatRequest::new(request.message);

            let result: Result<(StreamId, DeltaStream), _> =
                adapter.stream_chat(chat_request, trace_id).await;
            match result {
                Ok((_stream_id, mut delta_stream)) => {
                    while let Some(delta_result) = delta_stream.next().await {
                        match delta_result {
                            Ok(stream_delta) => {
                                let message_delta = match stream_delta.delta {
                                    DeltaType::Text { content } => MessageDelta {
                                        session_id: request.session_id.clone().unwrap_or_default(),
                                        message_id: message_id.clone(),
                                        sequence: 0,
                                        delta: MessageDeltaContent::Text { text: content },
                                        timestamp: stream_delta.timestamp,
                                    },
                                    DeltaType::ToolCall {
                                        id,
                                        name,
                                        arguments,
                                    } => MessageDelta {
                                        session_id: request.session_id.clone().unwrap_or_default(),
                                        message_id: message_id.clone(),
                                        sequence: 0,
                                        delta: MessageDeltaContent::ToolCall {
                                            tool_call_id: id,
                                            name: Some(name),
                                            args_json_fragment: arguments
                                                .map(|v| v.to_string())
                                                .unwrap_or_else(|| "{}".to_string()),
                                            done: false,
                                        },
                                        timestamp: stream_delta.timestamp,
                                    },
                                    DeltaType::Error(err) => {
                                        error!(
                                            code = err.code,
                                            message = err.message,
                                            "Stream error during chat delta processing"
                                        );
                                        continue;
                                    }
                                    DeltaType::StreamEnd { reason: _ } => {
                                        break;
                                    }
                                };

                                if let Ok(msg) = SubscriptionMessage::from_json(&message_delta)
                                    && sink.send(msg.clone()).await.is_err()
                                {
                                    break;
                                }
                            }
                            Err(e) => {
                                error!(error = %e, "Delta stream error");
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to start LLM stream");
                    let error_delta = MessageDelta {
                        session_id: request.session_id.clone().unwrap_or_default(),
                        message_id: message_id.clone(),
                        sequence: 0,
                        delta: MessageDeltaContent::Text {
                            text: format!("Error: Failed to start LLM stream - {e}"),
                        },
                        timestamp: chrono::Utc::now(),
                    };

                    if let Ok(msg) = SubscriptionMessage::from_json(&error_delta) {
                        let _ = sink.send(msg).await;
                    }
                }
            }
        } else {
            let error_delta = MessageDelta {
                session_id: request.session_id.clone().unwrap_or_default(),
                message_id: message_id.clone(),
                sequence: 0,
                delta: MessageDeltaContent::Text {
                    text: format!(
                        "Error: No LLM configured for agent '{}'. Model: '{}'",
                        agent_metadata.name, agent_metadata.model
                    ),
                },
                timestamp: chrono::Utc::now(),
            };

            if let Ok(msg) = SubscriptionMessage::from_json(&error_delta) {
                let _ = sink.send(msg).await;
            }
        }
    });

    timer.success();
    Ok(())
}
