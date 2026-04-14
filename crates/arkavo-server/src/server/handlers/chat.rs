use arkavo_protocol::auth::AuthBackend;
use arkavo_protocol::chat_session::ChatSessionManager;
use arkavo_protocol::metrics::{MetricsCollector, RpcTimer};
use arkavo_protocol::rate_limit::RateLimiter;
use arkavo_protocol::types::{ChatOpenRequest, ChatSession, UserMessage};
use jsonrpsee::types::ErrorObjectOwned;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

pub async fn handle_chat_open(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    auth_backend: &Arc<dyn AuthBackend>,
    chat_sessions: &Arc<ChatSessionManager>,
    request: ChatOpenRequest,
) -> Result<ChatSession, ErrorObjectOwned> {
    let timer = RpcTimer::new("chat_open".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    let auth = if let Some(token) = request.token {
        match auth_backend.validate_token(&token).await {
            Ok(auth) => Some(auth),
            Err(e) => {
                timer.error();
                return Err(ErrorObjectOwned::owned(
                    -32004,
                    format!("Authentication failed: {e}"),
                    None::<()>,
                ));
            }
        }
    } else {
        None
    };

    let session = chat_sessions.create_session(auth).await;

    timer.success();
    Ok(session)
}

pub async fn handle_chat_send(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    chat_sessions: &Arc<ChatSessionManager>,
    router: Option<&Arc<arkavo_router::Router>>,
    context_snapshot: &Arc<tokio::sync::RwLock<Option<serde_json::Value>>>,
    session_id: String,
    message: UserMessage,
) -> Result<(), ErrorObjectOwned> {
    info!(session.id = %session_id, content_len = message.content.len(), "chat_send called");
    let timer = RpcTimer::new("chat_send".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        warn!(session.id = %session_id, "Rate limit exceeded for chat_send");
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    // Introspection: @context returns conversation window snapshot
    if message.content.trim() == "@context" {
        let text = if let Some(ref snapshot) = *context_snapshot.read().await {
            snapshot.to_string()
        } else {
            r#"{"error":"no context snapshot available yet"}"#.to_string()
        };
        if let Err(e) = chat_sessions.push_system_delta(&session_id, text).await {
            warn!(session.id = %session_id, error = %e, "@context delta push failed");
        }
        timer.success();
        return Ok(());
    }

    // Preflight moderation: reject policy-violating messages before processing
    if let Some(router) = router
        && let Some(arkavo_router::ModerationResult::Block {
            policy_id, reason, ..
        }) = router.check_preflight(&message.content)
    {
        info!(
            session.id = %session_id,
            policy_id = %policy_id,
            "Chat message blocked by preflight policy"
        );
        timer.error();
        return Err(ErrorObjectOwned::owned(
            -32001,
            format!("Blocked by policy: {policy_id}"),
            Some(reason),
        ));
    }

    info!(session.id = %session_id, "Forwarding message to chat session manager");

    match chat_sessions.send_message(&session_id, message).await {
        Ok(()) => {
            info!(session.id = %session_id, "Message sent successfully to session");
            timer.success();
            Ok(())
        }
        Err(e) => {
            error!(session.id = %session_id, error = %e, "Failed to send message to session");
            timer.error();
            Err(ErrorObjectOwned::owned(
                e.to_json_rpc_code(),
                "Failed to send message",
                Some(e.to_string()),
            ))
        }
    }
}

pub async fn handle_chat_close(
    metrics: &Arc<MetricsCollector>,
    chat_sessions: &Arc<ChatSessionManager>,
    session_id: String,
) -> Result<(), ErrorObjectOwned> {
    let timer = RpcTimer::new("chat_close".to_string(), metrics.clone());

    match chat_sessions.close_session(&session_id).await {
        Ok(()) => {
            timer.success();
            Ok(())
        }
        Err(e) => {
            timer.error();
            Err(ErrorObjectOwned::owned(
                e.to_json_rpc_code(),
                "Failed to close session",
                Some(e.to_string()),
            ))
        }
    }
}

pub fn handle_chat_metrics_ack(
    metrics: &Arc<MetricsCollector>,
    session_id: &str,
    last_seq: u64,
) -> Result<(), ErrorObjectOwned> {
    debug!(
        session.id = %session_id,
        last_seq = last_seq,
        "Received delta acknowledgment"
    );

    let timer = RpcTimer::new("chat_metrics_ack".to_string(), metrics.clone());
    timer.success();
    Ok(())
}
