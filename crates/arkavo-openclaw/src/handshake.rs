use std::time::{SystemTime, UNIX_EPOCH};

use futures::{SinkExt, StreamExt};
use tokio::time::{Duration, timeout};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

use crate::device::DeviceIdentity;
use crate::protocol::{
    ChallengePayload, ClientInfo, ConnectAuth, ConnectParams, HelloOkPayload, OpenClawFrame,
    PROTOCOL_VERSION, ResponseFrame,
};

/// Configuration for the OpenClaw challenge-response handshake.
#[derive(Debug, Clone)]
pub struct HandshakeConfig {
    /// Gateway token (from `OPENCLAW_GATEWAY_TOKEN` env or explicit config).
    pub gateway_token: Option<String>,
    /// Previously issued device token (for reconnection without re-pairing).
    pub device_token: Option<String>,
    /// Client role declared during connect.
    pub role: String,
    /// Requested scopes.
    pub scopes: Vec<String>,
    /// Client identifier (must be a recognized OpenClaw client ID).
    pub client_id: String,
    /// Client mode (must be a recognized OpenClaw client mode).
    pub client_mode: String,
    /// Handshake timeout in milliseconds.
    pub timeout_ms: u64,
}

impl Default for HandshakeConfig {
    fn default() -> Self {
        Self {
            gateway_token: std::env::var("OPENCLAW_GATEWAY_TOKEN").ok(),
            device_token: None,
            role: "operator".to_string(),
            scopes: vec![
                "operator.admin".to_string(),
                "operator.read".to_string(),
                "operator.write".to_string(),
            ],
            client_id: "cli".to_string(),
            client_mode: "cli".to_string(),
            timeout_ms: 10_000,
        }
    }
}

/// Session information returned after a successful handshake.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub conn_id: Option<String>,
    pub protocol: u32,
    pub device_token: Option<String>,
    pub role: Option<String>,
    pub scopes: Vec<String>,
}

/// Result of a handshake attempt that may require device pairing approval.
#[derive(Debug)]
pub enum HandshakeOutcome {
    /// Handshake succeeded, session is ready.
    Connected(SessionInfo),
    /// Device pairing was requested, waiting for operator approval.
    /// The client should reconnect after approval.
    PairingRequested { request_id: Option<String> },
}

/// Run the client side of the OpenClaw handshake with optional device identity.
///
/// If `device` is provided, the challenge nonce is signed and included in the
/// connect request, enabling scoped access after device pairing approval.
pub async fn client_handshake<S>(
    ws: &mut S,
    config: &HandshakeConfig,
) -> Result<SessionInfo, HandshakeError>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + SinkExt<Message>
        + Unpin,
    <S as futures::Sink<Message>>::Error: std::fmt::Display,
{
    match client_handshake_with_device(ws, config, None).await? {
        HandshakeOutcome::Connected(session) => Ok(session),
        HandshakeOutcome::PairingRequested { .. } => Err(HandshakeError::Rejected(
            "device pairing required but no device identity provided".to_string(),
        )),
    }
}

/// Run the client handshake with device identity for scoped access.
///
/// Returns `HandshakeOutcome::PairingRequested` if the gateway needs operator
/// approval before granting scopes. The caller should prompt the user and
/// reconnect after approval.
pub async fn client_handshake_with_device<S>(
    ws: &mut S,
    config: &HandshakeConfig,
    device: Option<&DeviceIdentity>,
) -> Result<HandshakeOutcome, HandshakeError>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + SinkExt<Message>
        + Unpin,
    <S as futures::Sink<Message>>::Error: std::fmt::Display,
{
    let deadline = Duration::from_millis(config.timeout_ms);

    // Step 1: receive challenge event
    let challenge = timeout(deadline, receive_challenge(ws))
        .await
        .map_err(|_| HandshakeError::Timeout)??;

    debug!("received challenge nonce={}", challenge.nonce);

    // Step 2: build and send connect request
    let connect_id = uuid::Uuid::new_v4().to_string();
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let signed_device = device.map(|d| {
        let token_str = config
            .device_token
            .as_deref()
            .or(config.gateway_token.as_deref())
            .unwrap_or("");
        d.sign_challenge(&crate::device::ChallengeSignParams {
            client_id: &config.client_id,
            client_mode: &config.client_mode,
            role: &config.role,
            scopes: &config.scopes,
            signed_at_ms: now_ms,
            token: token_str,
            nonce: &challenge.nonce,
        })
    });

    let auth = {
        let auth = ConnectAuth {
            token: config.gateway_token.clone(),
            device_token: config.device_token.clone(),
            password: None,
        };
        if auth.token.is_none() && auth.device_token.is_none() && auth.password.is_none() {
            None
        } else {
            Some(auth)
        }
    };

    let params = ConnectParams {
        min_protocol: PROTOCOL_VERSION,
        max_protocol: PROTOCOL_VERSION,
        client: ClientInfo {
            id: config.client_id.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            platform: std::env::consts::OS.to_string(),
            mode: config.client_mode.clone(),
            display_name: None,
            instance_id: None,
        },
        role: Some(config.role.clone()),
        scopes: Some(config.scopes.clone()),
        caps: Some(vec![]),
        auth,
        device: signed_device,
        locale: Some("en-US".to_string()),
        user_agent: Some(format!("arkavo-openclaw/{}", env!("CARGO_PKG_VERSION"))),
    };

    let connect_frame = crate::protocol::connect_request(&connect_id, params)
        .map_err(|e| HandshakeError::Protocol(format!("connect request: {e}")))?;
    send_frame(ws, connect_frame).await?;
    debug!("sent connect request id={connect_id}");

    // Step 3: receive hello-ok or NOT_PAIRED response (skip events)
    let outcome = timeout(deadline, async {
        loop {
            let frame = receive_frame(ws).await?;
            match frame {
                OpenClawFrame::Response(ref r) if r.id == connect_id => {
                    if !r.ok
                        && let Some(ref err) = r.error
                        && err.code == "NOT_PAIRED"
                    {
                        let request_id = err
                            .data
                            .as_ref()
                            .and_then(|d| d.get("requestId"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        info!("device pairing requested (requestId={request_id:?})");
                        return Ok(HandshakeOutcome::PairingRequested { request_id });
                    }
                    let session = parse_hello_ok_response(r)?;
                    return Ok(HandshakeOutcome::Connected(session));
                }
                OpenClawFrame::Event(_) => {
                    debug!("skipping event during handshake");
                }
                other => {
                    return Err(HandshakeError::UnexpectedFrame(format!(
                        "expected res for {connect_id}, got {:?}",
                        std::mem::discriminant(&other)
                    )));
                }
            }
        }
    })
    .await
    .map_err(|_| HandshakeError::Timeout)??;

    match &outcome {
        HandshakeOutcome::Connected(s) => {
            debug!("handshake complete, conn_id={:?}", s.conn_id);
        }
        HandshakeOutcome::PairingRequested { request_id } => {
            info!("waiting for device pairing approval (requestId={request_id:?})");
        }
    }
    Ok(outcome)
}

async fn receive_challenge<S>(ws: &mut S) -> Result<ChallengePayload, HandshakeError>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let frame = receive_frame(ws).await?;
        match frame {
            OpenClawFrame::Event(ref e) if e.event == "connect.challenge" => {
                return serde_json::from_value(e.payload.clone())
                    .map_err(|e| HandshakeError::Protocol(format!("bad challenge payload: {e}")));
            }
            OpenClawFrame::Event(_) => {
                debug!("skipping non-challenge event");
            }
            other => {
                return Err(HandshakeError::UnexpectedFrame(format!(
                    "expected connect.challenge event, got {:?}",
                    std::mem::discriminant(&other)
                )));
            }
        }
    }
}

fn parse_hello_ok_response(r: &ResponseFrame) -> Result<SessionInfo, HandshakeError> {
    if !r.ok {
        let msg = r
            .error
            .as_ref()
            .map(|e| format!("{}: {}", e.code, e.message))
            .unwrap_or_else(|| "unknown error".to_string());
        return Err(HandshakeError::Rejected(msg));
    }

    let payload = r
        .payload
        .as_ref()
        .ok_or_else(|| HandshakeError::Protocol("hello-ok missing payload".to_string()))?;

    let ho: HelloOkPayload = serde_json::from_value(payload.clone())
        .map_err(|e| HandshakeError::Protocol(format!("bad hello-ok payload: {e}")))?;

    if ho.payload_type != "hello-ok" {
        return Err(HandshakeError::UnexpectedFrame(format!(
            "expected payload type hello-ok, got {}",
            ho.payload_type
        )));
    }

    Ok(SessionInfo {
        conn_id: ho.server.map(|s| s.conn_id),
        protocol: ho.protocol,
        device_token: ho.auth.as_ref().map(|a| a.device_token.clone()),
        role: ho.auth.as_ref().and_then(|a| a.role.clone()),
        scopes: ho.auth.and_then(|a| a.scopes).unwrap_or_default(),
    })
}

/// Run the server side of the OpenClaw handshake.
pub async fn server_handshake<S>(
    ws: &mut S,
    expected_token: Option<&str>,
    timeout_ms: u64,
) -> Result<(SessionInfo, String), HandshakeError>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + SinkExt<Message>
        + Unpin,
    <S as futures::Sink<Message>>::Error: std::fmt::Display,
{
    let deadline = Duration::from_millis(timeout_ms);

    // Step 1: send challenge event
    let nonce = uuid::Uuid::new_v4().to_string();
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let challenge = crate::protocol::challenge_event(&nonce, ts);
    send_frame(ws, challenge).await?;
    debug!("sent challenge nonce={nonce}");

    // Step 2: receive connect request
    let connect_frame = timeout(deadline, receive_frame(ws))
        .await
        .map_err(|_| HandshakeError::Timeout)??;

    let (connect_id, connect_params) = match connect_frame {
        OpenClawFrame::Request(ref r) if r.method == "connect" => {
            let params: ConnectParams = r
                .params
                .as_ref()
                .and_then(|p| serde_json::from_value(p.clone()).ok())
                .unwrap_or_else(|| ConnectParams {
                    min_protocol: PROTOCOL_VERSION,
                    max_protocol: PROTOCOL_VERSION,
                    client: ClientInfo {
                        id: "unknown".to_string(),
                        version: "unknown".to_string(),
                        platform: "unknown".to_string(),
                        mode: "cli".to_string(),
                        display_name: None,
                        instance_id: None,
                    },
                    role: None,
                    scopes: None,
                    caps: None,
                    auth: None,
                    device: None,
                    locale: None,
                    user_agent: None,
                });
            (r.id.clone(), params)
        }
        other => {
            return Err(HandshakeError::UnexpectedFrame(format!(
                "expected connect request, got {:?}",
                std::mem::discriminant(&other)
            )));
        }
    };

    // Validate auth
    if let Some(expected) = expected_token {
        let provided = connect_params
            .auth
            .as_ref()
            .and_then(|a| a.token.as_deref());
        if provided != Some(expected) {
            warn!("handshake auth failed: token mismatch");
            let err =
                crate::protocol::error_response(&connect_id, "NOT_LINKED", "authentication failed");
            let _ = send_frame(ws, err).await;
            return Err(HandshakeError::AuthFailed);
        }
    }

    // Step 3: send hello-ok response
    let conn_id = uuid::Uuid::new_v4().to_string();
    let hello = crate::protocol::hello_ok_response(&connect_id, PROTOCOL_VERSION, &conn_id)
        .map_err(|e| HandshakeError::Protocol(format!("hello-ok response: {e}")))?;
    send_frame(ws, hello).await?;
    debug!("handshake accepted, conn_id={conn_id}");

    let session = SessionInfo {
        conn_id: Some(conn_id),
        protocol: PROTOCOL_VERSION,
        device_token: None,
        role: connect_params.role,
        scopes: connect_params.scopes.unwrap_or_default(),
    };

    Ok((session, connect_id))
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
    fn default_config_reads_env() {
        let config = HandshakeConfig::default();
        assert_eq!(config.role, "operator");
        assert!(config.scopes.contains(&"operator.admin".to_string()));
        assert_eq!(config.client_id, "cli");
        assert_eq!(config.client_mode, "cli");
        assert_eq!(config.timeout_ms, 10_000);
    }

    #[test]
    fn session_info_defaults() {
        let session = SessionInfo {
            conn_id: None,
            protocol: 3,
            device_token: None,
            role: None,
            scopes: vec![],
        };
        assert_eq!(session.protocol, 3);
        assert!(session.scopes.is_empty());
    }

    #[test]
    fn parse_hello_ok_success() {
        let resp = ResponseFrame {
            id: "hs-1".to_string(),
            ok: true,
            payload: Some(serde_json::json!({
                "type": "hello-ok",
                "protocol": 3,
                "server": {"version": "dev", "connId": "abc"},
                "policy": {"tickIntervalMs": 30000}
            })),
            error: None,
        };
        let session = parse_hello_ok_response(&resp).unwrap();
        assert_eq!(session.protocol, 3);
        assert_eq!(session.conn_id.unwrap(), "abc");
    }

    #[test]
    fn parse_hello_ok_with_auth() {
        let resp = ResponseFrame {
            id: "hs-1".to_string(),
            ok: true,
            payload: Some(serde_json::json!({
                "type": "hello-ok",
                "protocol": 3,
                "auth": {
                    "deviceToken": "dtok_xyz",
                    "role": "operator",
                    "scopes": ["operator.read"]
                }
            })),
            error: None,
        };
        let session = parse_hello_ok_response(&resp).unwrap();
        assert_eq!(session.device_token.unwrap(), "dtok_xyz");
        assert_eq!(session.role.unwrap(), "operator");
        assert_eq!(session.scopes, vec!["operator.read"]);
    }

    #[test]
    fn parse_hello_ok_rejected() {
        let resp = ResponseFrame {
            id: "hs-1".to_string(),
            ok: false,
            payload: None,
            error: Some(crate::protocol::OpenClawError {
                code: "NOT_LINKED".to_string(),
                message: "bad token".to_string(),
                data: None,
            }),
        };
        let err = parse_hello_ok_response(&resp).unwrap_err();
        assert!(matches!(err, HandshakeError::Rejected(_)));
    }
}
