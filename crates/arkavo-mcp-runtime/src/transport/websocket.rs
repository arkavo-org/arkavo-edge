use super::{Transport, TransportError};
use async_trait::async_trait;
use futures::{SinkExt, StreamExt, stream::SplitSink};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::{Mutex, oneshot};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, warn};

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

type WsSink = SplitSink<WsStream, Message>;

/// WebSocket transport for MCP communication
pub struct WebSocketTransport {
    write: Arc<Mutex<WsSink>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    request_id: AtomicU64,
    connected: Arc<AtomicBool>,
}

impl WebSocketTransport {
    /// Create a new WebSocket transport
    pub async fn new(
        url: String,
        headers: HashMap<String, String>,
    ) -> Result<Self, TransportError> {
        // Build request with headers
        let request = url
            .parse::<http::Uri>()
            .map_err(|e| TransportError::ConnectionFailed(format!("Invalid URL: {e}")))?;

        let host = request
            .host()
            .ok_or_else(|| TransportError::ConnectionFailed("No host in URL".to_string()))?;

        let mut req_builder = http::Request::builder()
            .uri(&url)
            .header("Host", host)
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tokio_tungstenite::tungstenite::handshake::client::generate_key(),
            );

        for (key, value) in &headers {
            req_builder = req_builder.header(key.as_str(), value.as_str());
        }

        let request = req_builder.body(()).map_err(|e| {
            TransportError::ConnectionFailed(format!("Failed to build request: {e}"))
        })?;

        let (ws_stream, _) = connect_async(request).await.map_err(|e| {
            TransportError::ConnectionFailed(format!("WebSocket connect failed: {e}"))
        })?;

        let (write, read) = ws_stream.split();

        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let connected = Arc::new(AtomicBool::new(true));

        let transport = Self {
            write: Arc::new(Mutex::new(write)),
            pending: pending.clone(),
            request_id: AtomicU64::new(1),
            connected: connected.clone(),
        };

        // Spawn reader task
        let pending_clone = pending;
        let connected_flag = connected;
        tokio::spawn(async move {
            let mut read = read;
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(text)) => match serde_json::from_str::<Value>(&text) {
                        Ok(response) => {
                            if let Some(id) = response.get("id").and_then(|v| v.as_u64()) {
                                let mut pending_guard = pending_clone.lock().await;
                                if let Some(sender) = pending_guard.remove(&id) {
                                    let _ = sender.send(response);
                                }
                            } else {
                                debug!("Received WebSocket notification: {text}");
                            }
                        }
                        Err(e) => {
                            warn!("Failed to parse WebSocket message: {e} - {text}");
                        }
                    },
                    Ok(Message::Close(_)) => {
                        debug!("WebSocket closed by server");
                        connected_flag.store(false, Ordering::SeqCst);
                        break;
                    }
                    Ok(Message::Ping(data)) => {
                        debug!("Received ping: {:?}", data);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        error!("WebSocket error: {e}");
                        connected_flag.store(false, Ordering::SeqCst);
                        break;
                    }
                }
            }
        });

        Ok(transport)
    }
}

#[async_trait]
impl Transport for WebSocketTransport {
    async fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, TransportError> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(TransportError::ConnectionClosed);
        }

        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params.unwrap_or_else(|| json!({}))
        });

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        let message = Message::Text(serde_json::to_string(&request)?);
        {
            let mut write: tokio::sync::MutexGuard<'_, WsSink> = self.write.lock().await;
            write
                .send(message)
                .await
                .map_err(|e| TransportError::SendFailed(e.to_string()))?;
        }

        // Wait for response with timeout
        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(response)) => {
                if let Some(error) = response.get("error") {
                    Err(TransportError::ProtocolError(error.to_string()))
                } else {
                    Ok(response.get("result").cloned().unwrap_or(Value::Null))
                }
            }
            Ok(Err(_)) => Err(TransportError::ReceiveFailed("Channel closed".to_string())),
            Err(_) => {
                let mut pending = self.pending.lock().await;
                pending.remove(&id);
                Err(TransportError::Timeout)
            }
        }
    }

    async fn send_notification(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), TransportError> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(TransportError::ConnectionClosed);
        }

        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params.unwrap_or_else(|| json!({}))
        });

        let message = Message::Text(serde_json::to_string(&notification)?);
        let mut write: tokio::sync::MutexGuard<'_, WsSink> = self.write.lock().await;
        write
            .send(message)
            .await
            .map_err(|e| TransportError::SendFailed(e.to_string()))?;

        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.connected.store(false, Ordering::SeqCst);

        let mut write: tokio::sync::MutexGuard<'_, WsSink> = self.write.lock().await;
        let _ = write.send(Message::Close(None)).await;

        Ok(())
    }
}
