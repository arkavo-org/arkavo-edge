use std::time::{SystemTime, UNIX_EPOCH};

use futures::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use tokio::time::{Duration, timeout};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

use crate::protocol::{ChallengeFrame, ConnectAuth, ConnectFrame, HelloOkFrame, OpenClawFrame};

/// Configuration for the OpenClaw challenge-response handshake.
#[derive(Debug, Clone)]
pub struct HandshakeConfig {
    /// Gateway token (from `OPENCLAW_GATEWAY_TOKEN` env or explicit config).
    pub gateway_token: Option<String>,
    /// Client role declared during connect.
    pub role: String,
    /// Requested scopes.
    pub scope: Vec<String>,
    /// Handshake timeout in milliseconds.
    pub timeout_ms: u64,
}

impl Default for HandshakeConfig {
    fn default() -> Self {
        Self {
            gateway_token: std::env::var("OPENCLAW_GATEWAY_TOKEN").ok(),
            role: "agent".to_string(),
            scope: vec!["chat".to_string(), "tasks".to_string()],
            timeout_ms: 10_000,
        }
    }
}

/// Session information returned after a successful server-side handshake.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub device_token: String,
    pub role: Option<String>,
    pub scope: Vec<String>,
}

/// Run the client side of the OpenClaw handshake.
///
/// 1. Receive `connect.challenge` from server (nonce + timestamp).
/// 2. Send `connect` frame with auth token.
/// 3. Receive `hello-ok` with device token.
pub async fn client_handshake<S>(
    ws: &mut S,
    config: &HandshakeConfig,
) -> Result<HelloOkFrame, HandshakeError>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + SinkExt<Message>
        + Unpin,
    <S as futures::Sink<Message>>::Error: std::fmt::Display,
{
    let deadline = Duration::from_millis(config.timeout_ms);

    // Step 1: receive challenge
    let challenge = timeout(deadline, receive_frame(ws))
        .await
        .map_err(|_| HandshakeError::Timeout)??;

    let _challenge = match challenge {
        OpenClawFrame::Challenge(c) => {
            debug!("received challenge nonce={}", c.nonce);
            c
        }
        other => {
            return Err(HandshakeError::UnexpectedFrame(format!(
                "expected connect.challenge, got {:?}",
                std::mem::discriminant(&other)
            )));
        }
    };

    // Step 2: send connect with auth
    let connect = OpenClawFrame::Connect(ConnectFrame {
        auth: config.gateway_token.as_ref().map(|t| ConnectAuth {
            token: Some(t.clone()),
            password: None,
        }),
        role: Some(config.role.clone()),
        scope: Some(config.scope.clone()),
        min_protocol: Some("1".to_string()),
        max_protocol: Some("2".to_string()),
    });

    send_frame(ws, connect).await?;
    debug!("sent connect frame");

    // Step 3: receive hello-ok
    let hello = timeout(deadline, receive_frame(ws))
        .await
        .map_err(|_| HandshakeError::Timeout)??;

    match hello {
        OpenClawFrame::HelloOk(h) => {
            debug!("handshake complete, device_token={}", h.device_token);
            Ok(h)
        }
        OpenClawFrame::Error(e) => Err(HandshakeError::Rejected(e.message)),
        other => Err(HandshakeError::UnexpectedFrame(format!(
            "expected hello-ok, got {:?}",
            std::mem::discriminant(&other)
        ))),
    }
}

/// Run the server side of the OpenClaw handshake.
///
/// 1. Send `connect.challenge` with random nonce.
/// 2. Receive `connect` frame with auth.
/// 3. Validate auth, send `hello-ok` with device token.
pub async fn server_handshake<S>(
    ws: &mut S,
    expected_token: Option<&str>,
    timeout_ms: u64,
) -> Result<SessionInfo, HandshakeError>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + SinkExt<Message>
        + Unpin,
    <S as futures::Sink<Message>>::Error: std::fmt::Display,
{
    let deadline = Duration::from_millis(timeout_ms);

    // Step 1: send challenge
    let nonce = generate_nonce();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let challenge = OpenClawFrame::Challenge(ChallengeFrame {
        nonce: nonce.clone(),
        timestamp,
    });
    send_frame(ws, challenge).await?;
    debug!("sent challenge nonce={nonce}");

    // Step 2: receive connect
    let connect_frame = timeout(deadline, receive_frame(ws))
        .await
        .map_err(|_| HandshakeError::Timeout)??;

    let connect = match connect_frame {
        OpenClawFrame::Connect(c) => c,
        other => {
            return Err(HandshakeError::UnexpectedFrame(format!(
                "expected connect, got {:?}",
                std::mem::discriminant(&other)
            )));
        }
    };

    // Step 3: validate auth
    if let Some(expected) = expected_token {
        let provided = connect.auth.as_ref().and_then(|a| a.token.as_deref());
        if provided != Some(expected) {
            warn!("handshake auth failed: token mismatch");
            let err = OpenClawFrame::Error(crate::protocol::ErrorFrame {
                code: 4001,
                message: "authentication failed".to_string(),
                data: None,
            });
            let _ = send_frame(ws, err).await;
            return Err(HandshakeError::AuthFailed);
        }
    }

    // Step 4: generate device token and send hello-ok
    let device_token = generate_device_token(&nonce);
    let hello = OpenClawFrame::HelloOk(HelloOkFrame {
        device_token: device_token.clone(),
        server_id: None,
    });
    send_frame(ws, hello).await?;
    debug!("handshake accepted, issued device_token={device_token}");

    Ok(SessionInfo {
        device_token,
        role: connect.role,
        scope: connect.scope.unwrap_or_default(),
    })
}

fn generate_nonce() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = rng.r#gen();
    hex::encode(bytes)
}

fn generate_device_token(nonce: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(nonce.as_bytes());
    hasher.update(b"arkavo-openclaw-bridge");
    let result = hasher.finalize();
    format!("dtok_{}", hex::encode(&result[..16]))
}

async fn send_frame<S>(ws: &mut S, frame: OpenClawFrame) -> Result<(), HandshakeError>
where
    S: SinkExt<Message> + Unpin,
    <S as futures::Sink<Message>>::Error: std::fmt::Display,
{
    let json = serde_json::to_string(&frame)
        .map_err(|e| HandshakeError::Protocol(format!("serialize: {e}")))?;
    ws.send(Message::Text(json))
        .await
        .map_err(|e| HandshakeError::Transport(format!("send: {e}")))
}

async fn receive_frame<S>(ws: &mut S) -> Result<OpenClawFrame, HandshakeError>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let msg = ws
            .next()
            .await
            .ok_or(HandshakeError::ConnectionClosed)?
            .map_err(|e| HandshakeError::Transport(format!("receive: {e}")))?;
        match msg {
            Message::Text(text) => {
                return serde_json::from_str(&text)
                    .map_err(|e| HandshakeError::Protocol(format!("parse: {e}")));
            }
            Message::Close(_) => return Err(HandshakeError::ConnectionClosed),
            _ => {}
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
    #[error("Handshake timed out")]
    Timeout,
    #[error("Authentication failed")]
    AuthFailed,
    #[error("Handshake rejected: {0}")]
    Rejected(String),
    #[error("Unexpected frame: {0}")]
    UnexpectedFrame(String),
    #[error("Protocol error: {0}")]
    Protocol(String),
    #[error("Transport error: {0}")]
    Transport(String),
    #[error("Connection closed during handshake")]
    ConnectionClosed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_is_32_hex_chars() {
        let nonce = generate_nonce();
        assert_eq!(nonce.len(), 32);
        assert!(nonce.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn nonce_uniqueness() {
        let n1 = generate_nonce();
        let n2 = generate_nonce();
        assert_ne!(n1, n2);
    }

    #[test]
    fn device_token_is_deterministic_for_same_nonce() {
        let tok1 = generate_device_token("abc");
        let tok2 = generate_device_token("abc");
        assert_eq!(tok1, tok2);
        assert!(tok1.starts_with("dtok_"));
    }

    #[test]
    fn device_token_differs_for_different_nonces() {
        let tok1 = generate_device_token("abc");
        let tok2 = generate_device_token("def");
        assert_ne!(tok1, tok2);
    }

    #[test]
    fn default_config_reads_env() {
        // Should not panic even without the env var set
        let config = HandshakeConfig::default();
        assert_eq!(config.role, "agent");
        assert!(!config.scope.is_empty());
        assert_eq!(config.timeout_ms, 10_000);
    }

    // Integration handshake tests use real TCP + tokio-tungstenite
    // and live in server.rs tests (end_to_end_request_response).
}
