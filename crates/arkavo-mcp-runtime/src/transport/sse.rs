use super::{Transport, TransportError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::{Mutex, oneshot};
use tracing::{debug, warn};

/// SSE (Server-Sent Events) transport for HTTP-based MCP servers
pub struct SseTransport {
    url: String,
    headers: HashMap<String, String>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    request_id: AtomicU64,
    connected: AtomicBool,
    client: reqwest::Client,
    endpoint_url: Arc<Mutex<Option<String>>>,
}

impl SseTransport {
    /// Create a new SSE transport
    #[allow(clippy::unused_async)]
    pub async fn new(
        url: String,
        headers: HashMap<String, String>,
    ) -> Result<Self, TransportError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| TransportError::ConnectionFailed(e.to_string()))?;

        let transport = Self {
            url,
            headers,
            pending: Arc::new(Mutex::new(HashMap::new())),
            request_id: AtomicU64::new(1),
            connected: AtomicBool::new(true),
            client,
            endpoint_url: Arc::new(Mutex::new(None)),
        };

        Ok(transport)
    }

    /// Start the SSE connection and event listener
    async fn connect(&self) -> Result<String, TransportError> {
        // Check if we already have an endpoint
        {
            let endpoint = self.endpoint_url.lock().await;
            if let Some(ref url) = *endpoint {
                return Ok(url.clone());
            }
        }

        // Make initial GET request to establish SSE connection
        let mut request = self.client.get(&self.url);
        for (key, value) in &self.headers {
            request = request.header(key, value);
        }
        request = request.header("Accept", "text/event-stream");

        let response = request
            .send()
            .await
            .map_err(|e| TransportError::ConnectionFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(TransportError::ConnectionFailed(format!(
                "HTTP {}: {}",
                response.status(),
                response.status().canonical_reason().unwrap_or("Unknown")
            )));
        }

        // For SSE, the server typically sends an endpoint URL for posting messages
        // Parse the endpoint from the response or use the same URL
        let endpoint = format!("{}/message", self.url.trim_end_matches('/'));

        {
            let mut ep = self.endpoint_url.lock().await;
            *ep = Some(endpoint.clone());
        }

        // Start SSE event listener in background
        let pending = self.pending.clone();
        let url = self.url.clone();
        let headers = self.headers.clone();
        let client = self.client.clone();

        tokio::spawn(async move {
            if let Err(e) = Self::listen_events(client, url, headers, pending).await {
                warn!("SSE listener stopped: {e}");
            }
        });

        Ok(endpoint)
    }

    async fn listen_events(
        client: reqwest::Client,
        url: String,
        headers: HashMap<String, String>,
        pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    ) -> Result<(), TransportError> {
        let mut request = client.get(&url);
        for (key, value) in &headers {
            request = request.header(key, value);
        }
        request = request.header("Accept", "text/event-stream");

        let response = request
            .send()
            .await
            .map_err(|e| TransportError::ConnectionFailed(e.to_string()))?;

        let mut stream = response.bytes_stream();
        use futures::StreamExt;

        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| TransportError::ReceiveFailed(e.to_string()))?;
            let text = String::from_utf8_lossy(&chunk);
            buffer.push_str(&text);

            // Parse SSE events
            while let Some(pos) = buffer.find("\n\n") {
                let event = buffer[..pos].to_string();
                buffer = buffer[pos + 2..].to_string();

                // Parse the event data
                for line in event.lines() {
                    if let Some(data) = line.strip_prefix("data: ")
                        && let Ok(response) = serde_json::from_str::<Value>(data)
                    {
                        if let Some(id) = response.get("id").and_then(|v| v.as_u64()) {
                            let mut pending_guard = pending.lock().await;
                            if let Some(sender) = pending_guard.remove(&id) {
                                let _ = sender.send(response);
                            }
                        } else {
                            debug!("Received SSE notification: {data}");
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

#[async_trait]
impl Transport for SseTransport {
    async fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, TransportError> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(TransportError::ConnectionClosed);
        }

        let endpoint = self.connect().await?;

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

        // Send POST request
        let mut req = self.client.post(&endpoint);
        for (key, value) in &self.headers {
            req = req.header(key, value);
        }
        req = req.header("Content-Type", "application/json");

        let response = req
            .json(&request)
            .send()
            .await
            .map_err(|e| TransportError::SendFailed(e.to_string()))?;

        // For SSE, the response may come via the event stream or directly
        if response.status().is_success()
            && let Ok(body) = response.json::<Value>().await
            && (body.get("result").is_some() || body.get("error").is_some())
        {
            let mut pending = self.pending.lock().await;
            pending.remove(&id);

            if let Some(error) = body.get("error") {
                return Err(TransportError::ProtocolError(error.to_string()));
            }
            return Ok(body.get("result").cloned().unwrap_or(Value::Null));
        }

        // Wait for response via SSE with timeout
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

        let endpoint = self.connect().await?;

        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params.unwrap_or_else(|| json!({}))
        });

        let mut req = self.client.post(&endpoint);
        for (key, value) in &self.headers {
            req = req.header(key, value);
        }
        req = req.header("Content-Type", "application/json");

        req.json(&notification)
            .send()
            .await
            .map_err(|e| TransportError::SendFailed(e.to_string()))?;

        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.connected.store(false, Ordering::SeqCst);
        Ok(())
    }
}
