use std::net::SocketAddr;
use std::sync::Arc;

use arkavo_protocol::transport::{A2aRequest, A2aResponse};
use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

use crate::handshake::{self, SessionInfo};
use crate::protocol::OpenClawFrame;
use crate::translator;

/// Trait for dispatching translated A2A requests to the local handler.
///
/// Implementors bridge between the OpenClaw listener and the actual A2A
/// processing pipeline (e.g., `A2aRpcImpl` handlers).
#[async_trait]
pub trait OpenClawDispatcher: Send + Sync {
    async fn dispatch(&self, request: A2aRequest) -> A2aResponse;
}

/// Configuration for the OpenClaw WebSocket listener.
#[derive(Debug, Clone)]
pub struct ListenerConfig {
    /// Address to bind (default: 127.0.0.1:18789).
    pub bind_addr: SocketAddr,
    /// Expected gateway token for authenticating connecting clients.
    /// If `None`, authentication is skipped.
    pub expected_token: Option<String>,
    /// Handshake timeout in milliseconds.
    pub handshake_timeout_ms: u64,
}

impl Default for ListenerConfig {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 18789)),
            expected_token: std::env::var("OPENCLAW_GATEWAY_TOKEN").ok(),
            handshake_timeout_ms: 10_000,
        }
    }
}

/// WebSocket server that accepts OpenClaw clients and translates
/// their requests into A2A JSON-RPC for local dispatch.
pub struct OpenClawListener {
    config: ListenerConfig,
    dispatcher: Arc<dyn OpenClawDispatcher>,
}

impl OpenClawListener {
    pub fn new(config: ListenerConfig, dispatcher: Arc<dyn OpenClawDispatcher>) -> Self {
        Self { config, dispatcher }
    }

    /// Start accepting connections. Returns a handle to the listener task
    /// and the actual bound address (useful when binding to port 0).
    pub async fn start(self) -> Result<(JoinHandle<()>, SocketAddr), std::io::Error> {
        let listener = TcpListener::bind(self.config.bind_addr).await?;
        let local_addr = listener.local_addr()?;
        info!("OpenClaw listener started on {local_addr}");

        let config = Arc::new(self.config);
        let dispatcher = self.dispatcher;

        let handle = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, peer_addr)) => {
                        debug!("OpenClaw connection from {peer_addr}");
                        let cfg = config.clone();
                        let disp = dispatcher.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(stream, cfg, disp).await {
                                warn!("OpenClaw connection from {peer_addr} ended: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        error!("Failed to accept OpenClaw connection: {e}");
                    }
                }
            }
        });

        Ok((handle, local_addr))
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    config: Arc<ListenerConfig>,
    dispatcher: Arc<dyn OpenClawDispatcher>,
) -> Result<(), ConnectionError> {
    // Upgrade to WebSocket
    let mut ws = accept_async(stream)
        .await
        .map_err(|e| ConnectionError::WebSocket(format!("upgrade failed: {e}")))?;

    // Run server-side handshake
    let session = handshake::server_handshake(
        &mut ws,
        config.expected_token.as_deref(),
        config.handshake_timeout_ms,
    )
    .await
    .map_err(|e| ConnectionError::Handshake(e.to_string()))?;

    info!(
        "OpenClaw client authenticated: role={:?}, scopes={:?}",
        session.role, session.scope
    );

    // Enter message loop
    message_loop(&mut ws, &dispatcher, &session).await
}

async fn message_loop<S>(
    ws: &mut S,
    dispatcher: &Arc<dyn OpenClawDispatcher>,
    _session: &SessionInfo,
) -> Result<(), ConnectionError>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + SinkExt<Message>
        + Unpin,
    <S as futures::Sink<Message>>::Error: std::fmt::Display,
{
    while let Some(msg_result) = ws.next().await {
        let msg = msg_result.map_err(|e| ConnectionError::WebSocket(format!("read: {e}")))?;

        match msg {
            Message::Text(text) => {
                let frame: OpenClawFrame = match serde_json::from_str(&text) {
                    Ok(f) => f,
                    Err(e) => {
                        warn!("invalid frame: {e}");
                        continue;
                    }
                };

                match frame {
                    OpenClawFrame::Request(req) => {
                        let oc_id = req.id.clone();
                        match translator::openclaw_req_to_a2a(&req) {
                            Ok(a2a_req) => {
                                let a2a_resp = dispatcher.dispatch(a2a_req).await;
                                let oc_resp =
                                    translator::a2a_response_to_openclaw(&oc_id, &a2a_resp);
                                let resp_frame = OpenClawFrame::Response(oc_resp);
                                let json = serde_json::to_string(&resp_frame).map_err(|e| {
                                    ConnectionError::Protocol(format!("serialize: {e}"))
                                })?;
                                ws.send(Message::Text(json)).await.map_err(|e| {
                                    ConnectionError::WebSocket(format!("send: {e}"))
                                })?;
                            }
                            Err(e) => {
                                let err_resp =
                                    OpenClawFrame::Response(crate::protocol::ResponseFrame {
                                        id: oc_id,
                                        result: None,
                                        error: Some(crate::protocol::OpenClawError {
                                            code: -32600,
                                            message: format!("translation error: {e}"),
                                            data: None,
                                        }),
                                    });
                                let json = serde_json::to_string(&err_resp).map_err(|e| {
                                    ConnectionError::Protocol(format!("serialize: {e}"))
                                })?;
                                ws.send(Message::Text(json)).await.map_err(|e| {
                                    ConnectionError::WebSocket(format!("send: {e}"))
                                })?;
                            }
                        }
                    }
                    OpenClawFrame::Event(ev) => {
                        debug!("received event: topic={}", ev.topic);
                        // Events from clients are acknowledged but not dispatched
                    }
                    _ => {
                        debug!("ignoring non-request frame in message loop");
                    }
                }
            }
            Message::Close(_) => {
                info!("OpenClaw client disconnected");
                break;
            }
            Message::Ping(data) => {
                ws.send(Message::Pong(data))
                    .await
                    .map_err(|e| ConnectionError::WebSocket(format!("pong: {e}")))?;
            }
            _ => {}
        }
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum ConnectionError {
    #[error("WebSocket error: {0}")]
    WebSocket(String),
    #[error("Handshake error: {0}")]
    Handshake(String),
    #[error("Protocol error: {0}")]
    Protocol(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_protocol::transport::JsonRpcError;

    struct EchoDispatcher;

    #[async_trait]
    impl OpenClawDispatcher for EchoDispatcher {
        async fn dispatch(&self, request: A2aRequest) -> A2aResponse {
            A2aResponse::Success {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: serde_json::json!({
                    "echo": request.method,
                    "params": request.params
                }),
            }
        }
    }

    struct ErrorDispatcher;

    #[async_trait]
    impl OpenClawDispatcher for ErrorDispatcher {
        async fn dispatch(&self, request: A2aRequest) -> A2aResponse {
            A2aResponse::Error {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                error: JsonRpcError {
                    code: -32601,
                    message: "not implemented".to_string(),
                    data: None,
                },
            }
        }
    }

    #[tokio::test]
    async fn listener_binds_and_starts() {
        let config = ListenerConfig {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            expected_token: None,
            handshake_timeout_ms: 5000,
        };
        let listener = OpenClawListener::new(config, Arc::new(EchoDispatcher));
        let (handle, addr) = listener.start().await.unwrap();
        assert_ne!(addr.port(), 0);
        handle.abort();
    }

    #[tokio::test]
    async fn end_to_end_request_response() {
        let config = ListenerConfig {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            expected_token: None,
            handshake_timeout_ms: 5000,
        };
        let listener = OpenClawListener::new(config, Arc::new(EchoDispatcher));
        let (handle, addr) = listener.start().await.unwrap();

        // Connect as a client via raw tokio-tungstenite
        let url = format!("ws://127.0.0.1:{}", addr.port());
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        // Server sends challenge
        let challenge_msg = ws.next().await.unwrap().unwrap();
        let challenge_text = match challenge_msg {
            Message::Text(t) => t,
            other => panic!("expected text, got {other:?}"),
        };
        let challenge: OpenClawFrame = serde_json::from_str(&challenge_text).unwrap();
        assert!(matches!(challenge, OpenClawFrame::Challenge(_)));

        // Send connect (no auth required)
        let connect = OpenClawFrame::Connect(crate::protocol::ConnectFrame {
            auth: None,
            role: Some("test".to_string()),
            scope: None,
            min_protocol: None,
            max_protocol: None,
        });
        ws.send(Message::Text(serde_json::to_string(&connect).unwrap()))
            .await
            .unwrap();

        // Receive hello-ok
        let hello_msg = ws.next().await.unwrap().unwrap();
        let hello_text = match hello_msg {
            Message::Text(t) => t,
            other => panic!("expected text, got {other:?}"),
        };
        let hello: OpenClawFrame = serde_json::from_str(&hello_text).unwrap();
        assert!(matches!(hello, OpenClawFrame::HelloOk(_)));

        // Send a chat request
        let req = OpenClawFrame::Request(crate::protocol::RequestFrame {
            id: "test-1".to_string(),
            method: "chat".to_string(),
            params: Some(serde_json::json!({"content": "hello"})),
        });
        ws.send(Message::Text(serde_json::to_string(&req).unwrap()))
            .await
            .unwrap();

        // Receive response
        let resp_msg = ws.next().await.unwrap().unwrap();
        let resp_text = match resp_msg {
            Message::Text(t) => t,
            other => panic!("expected text, got {other:?}"),
        };
        let resp: OpenClawFrame = serde_json::from_str(&resp_text).unwrap();
        match resp {
            OpenClawFrame::Response(r) => {
                assert_eq!(r.id, "test-1");
                assert!(r.error.is_none());
                assert_eq!(r.result.as_ref().unwrap()["echo"], "message/send");
            }
            other => panic!("expected Response, got {other:?}"),
        }

        ws.close(None).await.unwrap();
        handle.abort();
    }

    #[tokio::test]
    async fn error_dispatch_returns_openclaw_error() {
        let config = ListenerConfig {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            expected_token: None,
            handshake_timeout_ms: 5000,
        };
        let listener = OpenClawListener::new(config, Arc::new(ErrorDispatcher));
        let (handle, addr) = listener.start().await.unwrap();

        let url = format!("ws://127.0.0.1:{}", addr.port());
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        // Handshake
        let _ = ws.next().await; // challenge
        let connect = OpenClawFrame::Connect(crate::protocol::ConnectFrame {
            auth: None,
            role: None,
            scope: None,
            min_protocol: None,
            max_protocol: None,
        });
        ws.send(Message::Text(serde_json::to_string(&connect).unwrap()))
            .await
            .unwrap();
        let _ = ws.next().await; // hello-ok

        // Send request
        let req = OpenClawFrame::Request(crate::protocol::RequestFrame {
            id: "err-1".to_string(),
            method: "chat".to_string(),
            params: Some(serde_json::json!({"content": "test"})),
        });
        ws.send(Message::Text(serde_json::to_string(&req).unwrap()))
            .await
            .unwrap();

        // Response should have error
        let resp_msg = ws.next().await.unwrap().unwrap();
        let resp: OpenClawFrame = serde_json::from_str(&match resp_msg {
            Message::Text(t) => t,
            _ => panic!(),
        })
        .unwrap();
        match resp {
            OpenClawFrame::Response(r) => {
                assert_eq!(r.id, "err-1");
                assert!(r.error.is_some());
                assert_eq!(r.error.unwrap().code, -32601);
            }
            _ => panic!("expected Response"),
        }

        ws.close(None).await.unwrap();
        handle.abort();
    }
}
