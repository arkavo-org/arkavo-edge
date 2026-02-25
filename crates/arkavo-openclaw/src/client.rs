use std::sync::Arc;
use std::time::Duration;

use arkavo_protocol::error::A2aError;
use arkavo_protocol::transport::{
    A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, TransportConfig,
};
use async_trait::async_trait;
use dashmap::DashMap;
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use rustls::{ClientConfig, RootCertStore};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::{RwLock, broadcast, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async_tls_with_config,
};
use tracing::{debug, error, info, warn};

use crate::device::DeviceIdentity;
use crate::handshake::{HandshakeConfig, HandshakeOutcome, client_handshake_with_device};
use crate::protocol::{OpenClawFrame, ResponseFrame};
use crate::translator;

/// An event received from the OpenClaw gateway's event stream.
#[derive(Debug, Clone)]
pub struct OpenClawEvent {
    pub event: String,
    pub payload: Value,
    pub seq: Option<u64>,
}

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsSink = SplitSink<WsStream, Message>;
type WsReader = SplitStream<WsStream>;

struct OpenClawConnection {
    writer: Arc<RwLock<WsSink>>,
    reader_task: JoinHandle<()>,
}

/// WebSocket transport that connects to an OpenClaw gateway.
///
/// Implements the `A2aTransport` trait, performing the OpenClaw challenge-response
/// handshake on connect and translating A2A JSON-RPC requests into OpenClaw frames.
pub struct OpenClawTransport {
    transport_config: TransportConfig,
    handshake_config: HandshakeConfig,
    device: Option<DeviceIdentity>,
    connection: Arc<RwLock<Option<OpenClawConnection>>>,
    endpoint: Arc<RwLock<Option<A2aEndpoint>>>,
    /// Maps OpenClaw request ID (string) -> oneshot sender for response routing.
    pending_requests: Arc<DashMap<String, (uuid::Uuid, oneshot::Sender<ResponseFrame>)>>,
    /// Broadcast channel for server-pushed events (chat deltas, presence, etc.).
    event_tx: broadcast::Sender<OpenClawEvent>,
}

impl OpenClawTransport {
    pub fn new(transport_config: TransportConfig, handshake_config: HandshakeConfig) -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            transport_config,
            handshake_config,
            device: None,
            connection: Arc::new(RwLock::new(None)),
            endpoint: Arc::new(RwLock::new(None)),
            pending_requests: Arc::new(DashMap::new()),
            event_tx,
        }
    }

    /// Subscribe to the server-pushed event stream.
    ///
    /// Returns a receiver that yields `OpenClawEvent` for each event frame
    /// received from the gateway (chat deltas, presence updates, etc.).
    pub fn subscribe_events(&self) -> broadcast::Receiver<OpenClawEvent> {
        self.event_tx.subscribe()
    }

    /// Create transport with a device identity for scoped access.
    pub fn with_device(mut self, device: DeviceIdentity) -> Self {
        self.device = Some(device);
        self
    }

    /// Send a chat message and get back the run ID and an event receiver for streaming deltas.
    ///
    /// Sends a `chat.send` request with the given session key and message, then returns
    /// the `runId` from the immediate response along with a broadcast receiver. The caller
    /// should read events from the receiver and filter by matching `runId` in the payload.
    pub async fn send_chat(
        &self,
        session_key: &str,
        message: &str,
    ) -> anyhow::Result<(String, broadcast::Receiver<OpenClawEvent>)> {
        let rx = self.subscribe_events();

        let idempotency_key = uuid::Uuid::new_v4().to_string();
        let request = A2aRequest::new(
            "message/send",
            serde_json::json!({
                "sessionKey": session_key,
                "message": message,
                "idempotencyKey": idempotency_key,
            }),
        );

        let response = self.send_request(request).await?;

        let run_id = match &response {
            A2aResponse::Success { result, .. } => result
                .get("runId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            A2aResponse::Error { error, .. } => {
                return Err(
                    A2aError::WebSocket(format!("chat.send failed: {}", error.message)).into(),
                );
            }
        };

        Ok((run_id, rx))
    }

    fn build_tls_connector(&self) -> Result<Connector, A2aError> {
        let mut root_store = RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Ok(Connector::Rustls(Arc::new(config)))
    }

    async fn connect_websocket(&self, url: &str) -> Result<WsStream, A2aError> {
        let connector = if url.starts_with("wss://") {
            self.build_tls_connector()?
        } else {
            Connector::Plain
        };

        let timeout_duration = Duration::from_millis(self.transport_config.timeout_ms);

        let (ws_stream, _) = timeout(
            timeout_duration,
            connect_async_tls_with_config(url, None, false, Some(connector)),
        )
        .await
        .map_err(|_| A2aError::Timeout(self.transport_config.timeout_ms))?
        .map_err(|e| A2aError::ConnectionFailed(format!("OpenClaw WS connection failed: {e}")))?;

        Ok(ws_stream)
    }

    fn start_reader_task(
        mut reader: WsReader,
        pending: Arc<DashMap<String, (uuid::Uuid, oneshot::Sender<ResponseFrame>)>>,
        event_tx: broadcast::Sender<OpenClawEvent>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(msg_result) = reader.next().await {
                match msg_result {
                    Ok(Message::Text(text)) => match serde_json::from_str::<OpenClawFrame>(&text) {
                        Ok(OpenClawFrame::Response(resp)) => {
                            if let Some((_, (_, sender))) = pending.remove(&resp.id) {
                                let _ = sender.send(resp);
                            } else {
                                warn!("response for unknown id: {}", resp.id);
                            }
                        }
                        Ok(OpenClawFrame::Event(ev)) => {
                            debug!("event: {} seq={:?}", ev.event, ev.seq);
                            let _ = event_tx.send(OpenClawEvent {
                                event: ev.event,
                                payload: ev.payload,
                                seq: ev.seq,
                            });
                        }
                        Ok(_) => {
                            debug!("ignoring non-response frame");
                        }
                        Err(e) => {
                            warn!("failed to parse frame: {e}");
                        }
                    },
                    Ok(Message::Close(_)) => {
                        info!("OpenClaw connection closed by server");
                        break;
                    }
                    Err(e) => {
                        error!("OpenClaw WS read error: {e}");
                        break;
                    }
                    _ => {}
                }
            }
            pending.clear();
        })
    }
}

#[async_trait]
impl A2aTransport for OpenClawTransport {
    async fn connect(&self, endpoint: &A2aEndpoint) -> anyhow::Result<()> {
        info!("Connecting to OpenClaw gateway: {}", endpoint.url);

        if !endpoint.url.starts_with("ws://") && !endpoint.url.starts_with("wss://") {
            return Err(A2aError::InvalidEndpoint(format!(
                "OpenClaw URL must start with ws:// or wss://, got: {}",
                endpoint.url
            ))
            .into());
        }

        if self.transport_config.tls_config.require_tls && endpoint.url.starts_with("ws://") {
            return Err(A2aError::Tls(
                "TLS required but URL uses ws:// instead of wss://".to_string(),
            )
            .into());
        }

        let mut ws_stream = self.connect_websocket(&endpoint.url).await?;

        let outcome = client_handshake_with_device(
            &mut ws_stream,
            &self.handshake_config,
            self.device.as_ref(),
        )
        .await
        .map_err(|e| A2aError::AuthenticationFailed(format!("OpenClaw handshake: {e}")))?;

        let session = match outcome {
            HandshakeOutcome::Connected(session) => session,
            HandshakeOutcome::PairingRequested { request_id } => {
                return Err(A2aError::AuthenticationFailed(format!(
                    "device pairing required (requestId={request_id:?}), approve with: openclaw devices approve"
                ))
                .into());
            }
        };

        info!("OpenClaw handshake complete, conn_id={:?}", session.conn_id);

        let (writer, reader) = ws_stream.split();
        let reader_task =
            Self::start_reader_task(reader, self.pending_requests.clone(), self.event_tx.clone());

        let connection = OpenClawConnection {
            writer: Arc::new(RwLock::new(writer)),
            reader_task,
        };

        *self.connection.write().await = Some(connection);
        *self.endpoint.write().await = Some(endpoint.clone());

        Ok(())
    }

    async fn send_request(&self, request: A2aRequest) -> anyhow::Result<A2aResponse> {
        debug!(
            "Sending OpenClaw request: method={}, id={}",
            request.method, request.id
        );

        let writer = {
            let guard = self.connection.read().await;
            guard.as_ref().ok_or(A2aError::NotConnected)?.writer.clone()
        };

        // Translate A2A request to OpenClaw frame
        let oc_frame = translator::a2a_to_openclaw_req(&request);
        let oc_id = oc_frame.id.clone();
        let a2a_id = request.id;

        // Register pending request
        let (tx, rx) = oneshot::channel();
        self.pending_requests.insert(oc_id.clone(), (a2a_id, tx));

        // Serialize and send
        let frame = OpenClawFrame::Request(oc_frame);
        let json = serde_json::to_string(&frame)?;

        writer
            .write()
            .await
            .send(Message::Text(json))
            .await
            .map_err(|e| A2aError::WebSocket(format!("send failed: {e}")))?;

        // Wait for response with timeout
        let resp = timeout(Duration::from_millis(self.transport_config.timeout_ms), rx)
            .await
            .map_err(|_| {
                self.pending_requests.remove(&oc_id);
                A2aError::Timeout(self.transport_config.timeout_ms)
            })?
            .map_err(|_| A2aError::WebSocket("response channel closed".to_string()))?;

        Ok(translator::openclaw_response_to_a2a(a2a_id, &resp))
    }

    async fn close(&self) -> anyhow::Result<()> {
        info!("Closing OpenClaw connection");

        let conn = self.connection.write().await.take();
        if let Some(conn) = conn {
            if let Ok(mut writer) = conn.writer.try_write() {
                let _ = writer.send(Message::Close(None)).await;
            }
            conn.reader_task.abort();
        }

        *self.endpoint.write().await = None;
        self.pending_requests.clear();
        Ok(())
    }

    fn is_connected(&self) -> bool {
        let guard = futures::executor::block_on(self.connection.read());
        guard.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn transport_creation() {
        let transport =
            OpenClawTransport::new(TransportConfig::default(), HandshakeConfig::default());
        assert!(!transport.is_connected());
    }

    #[tokio::test]
    async fn invalid_url_rejected() {
        let transport =
            OpenClawTransport::new(TransportConfig::default(), HandshakeConfig::default());
        let endpoint = A2aEndpoint {
            url: "http://localhost:18789".to_string(),
            agent_id: "test".to_string(),
            public_key: None,
        };
        let result = transport.connect(&endpoint).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("ws://"));
    }

    #[tokio::test]
    async fn tls_required_rejects_ws() {
        let mut config = TransportConfig::default();
        config.tls_config.require_tls = true;
        let transport = OpenClawTransport::new(config, HandshakeConfig::default());
        let endpoint = A2aEndpoint {
            url: "ws://localhost:18789".to_string(),
            agent_id: "test".to_string(),
            public_key: None,
        };
        let result = transport.connect(&endpoint).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("TLS required"));
    }

    #[tokio::test]
    async fn send_without_connect_fails() {
        let transport =
            OpenClawTransport::new(TransportConfig::default(), HandshakeConfig::default());
        let req = A2aRequest::new("test", serde_json::json!({}));
        let result = transport.send_request(req).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not connected"));
    }
}
