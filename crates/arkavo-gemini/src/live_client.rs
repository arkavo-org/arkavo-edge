use crate::error::{GeminiError, Result};
use crate::types::{
    ClientContent, ClientMessage, FunctionCall, FunctionDeclaration, GenerationConfig, SetupConfig,
    Tool, ToolCall, ToolResponse,
};
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::{RwLock, mpsc};
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tracing::{debug, error, info, warn};
use url::Url;

/// Default WebSocket endpoint for the Gemini Live (BidiGenerateContent) API.
///
/// Gemini 3.5 makes the WebSocket Live API the recommended low-latency streaming
/// surface — text, audio, and video flow over a single stateful socket. Use
/// this constant when wiring custom clients; the bundled `LiveSessionClient`
/// targets it automatically.
pub const GEMINI_WS_ENDPOINT: &str = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";

/// Default model identifier for live WebSocket sessions.
pub const DEFAULT_LIVE_MODEL: &str = "gemini-3.5-flash";

const MAX_RECONNECT_ATTEMPTS: u32 = 5;
const INITIAL_BACKOFF_MS: u64 = 1000;

/// Response modalities the Live API can emit. Default is `Text` to match the
/// general text-streaming use case; audio/video sessions opt in explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveModality {
    Text,
    Audio,
}

impl LiveModality {
    fn as_str(self) -> &'static str {
        match self {
            Self::Text => "TEXT",
            Self::Audio => "AUDIO",
        }
    }
}

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct LiveSessionClient {
    api_key: String,
    model: String,
    tools: Vec<Value>,
    modality: LiveModality,
    ws_stream: Arc<RwLock<Option<WsStream>>>,
    tool_call_tx: mpsc::UnboundedSender<Vec<FunctionCall>>,
    tool_call_rx: Arc<RwLock<Option<mpsc::UnboundedReceiver<Vec<FunctionCall>>>>>,
    connected: Arc<AtomicBool>,
}

impl LiveSessionClient {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::new_with_tools(api_key, model, vec![])
    }

    /// Construct a client pinned to the default Gemini Live model
    /// (`DEFAULT_LIVE_MODEL`) — the WebSocket-based streaming entry point that
    /// Gemini 3.5 promotes as the default for low-latency interaction.
    pub fn new_default(api_key: impl Into<String>) -> Self {
        Self::new(api_key, DEFAULT_LIVE_MODEL)
    }

    /// Build a Live client from `GEMINI_API_KEY` using the default model.
    #[allow(clippy::result_large_err)]
    pub fn try_from_env() -> Result<Self> {
        let api_key = std::env::var("GEMINI_API_KEY")
            .map_err(|_| GeminiError::Config("GEMINI_API_KEY not set for Live API".to_string()))?;
        let model =
            std::env::var("GEMINI_MODEL").unwrap_or_else(|_| DEFAULT_LIVE_MODEL.to_string());
        Ok(Self::new(api_key, model))
    }

    pub fn new_with_tools(
        api_key: impl Into<String>,
        model: impl Into<String>,
        tools: Vec<Value>,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            api_key: api_key.into(),
            model: model.into(),
            tools,
            modality: LiveModality::Text,
            ws_stream: Arc::new(RwLock::new(None)),
            tool_call_tx: tx,
            tool_call_rx: Arc::new(RwLock::new(Some(rx))),
            connected: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Override the response modality (default `Text`). Must be set before
    /// `connect()` since the setup frame is sent at that point.
    pub fn with_modality(mut self, modality: LiveModality) -> Self {
        self.modality = modality;
        self
    }

    pub async fn connect(&self) -> Result<()> {
        let mut attempt = 0;
        let mut backoff = INITIAL_BACKOFF_MS;

        while attempt < MAX_RECONNECT_ATTEMPTS {
            match self.try_connect().await {
                Ok(()) => {
                    info!("Successfully connected to Gemini Live API");
                    return Ok(());
                }
                Err(e) => {
                    attempt += 1;
                    if attempt >= MAX_RECONNECT_ATTEMPTS {
                        error!("Failed to connect after {} attempts", attempt);
                        return Err(e);
                    }
                    warn!(
                        "Connection attempt {} failed: {}. Retrying in {}ms...",
                        attempt, e, backoff
                    );
                    sleep(Duration::from_millis(backoff)).await;
                    backoff *= 2;
                }
            }
        }

        Err(GeminiError::ConnectionTimeout(backoff))
    }

    async fn try_connect(&self) -> Result<()> {
        let url_with_key = format!("{}?key={}", GEMINI_WS_ENDPOINT, self.api_key);
        let _url = Url::parse(&url_with_key)?;

        debug!("Connecting to Gemini WebSocket endpoint");
        let (ws_stream, _) = connect_async(&url_with_key).await?;

        let mut stream_guard = self.ws_stream.write().await;
        *stream_guard = Some(ws_stream);
        drop(stream_guard);

        self.send_setup().await?;
        self.connected.store(true, Ordering::Relaxed);

        self.start_receiver_task();

        Ok(())
    }

    async fn send_setup(&self) -> Result<()> {
        let tools = if self.tools.is_empty() {
            None
        } else {
            let function_declarations: Vec<FunctionDeclaration> = self
                .tools
                .iter()
                .map(|t| FunctionDeclaration {
                    name: t["name"].as_str().unwrap_or("unknown").to_string(),
                    description: t["description"].as_str().unwrap_or("").to_string(),
                    parameters: t["parameters"].clone(),
                })
                .collect();

            Some(vec![Tool {
                function_declarations,
            }])
        };

        let setup = SetupConfig {
            model: self.model.clone(),
            generation_config: Some(GenerationConfig {
                response_modalities: vec![self.modality.as_str().to_string()],
                temperature: None,
                max_output_tokens: None,
            }),
            tools,
        };

        self.send_message(ClientMessage::Setup { setup }).await
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn send_message(&self, message: ClientMessage) -> Result<()> {
        let json = serde_json::to_string(&message)?;
        info!("Sending message: {}", json);

        let mut stream_guard = self.ws_stream.write().await;
        let stream = stream_guard.as_mut().ok_or(GeminiError::NotConnected)?;

        stream.send(Message::Text(json)).await?;
        Ok(())
    }

    #[allow(clippy::significant_drop_tightening)]
    fn start_receiver_task(&self) {
        let ws_stream = self.ws_stream.clone();
        let tool_call_tx = self.tool_call_tx.clone();
        let connected = self.connected.clone();

        tokio::spawn(async move {
            info!("Receiver task started");
            loop {
                let message = {
                    let mut stream_guard = ws_stream.write().await;
                    let stream = match stream_guard.as_mut() {
                        Some(s) => s,
                        None => {
                            debug!("Receiver task: stream not connected");
                            break;
                        }
                    };

                    debug!("Waiting for next message...");
                    match stream.next().await {
                        Some(Ok(msg)) => {
                            debug!("Received websocket message type: {:?}", msg);
                            msg
                        }
                        Some(Err(e)) => {
                            error!("WebSocket error: {}", e);
                            connected.store(false, Ordering::Relaxed);
                            break;
                        }
                        None => {
                            info!("WebSocket stream ended");
                            connected.store(false, Ordering::Relaxed);
                            break;
                        }
                    }
                };

                let should_break = match message {
                    Message::Text(text) => {
                        info!("Received message: {}", text);
                        Self::process_json_message(&text, &tool_call_tx)
                    }
                    Message::Binary(bytes) => match String::from_utf8(bytes) {
                        Ok(json_str) => {
                            info!("Received message: {}", json_str);
                            Self::process_json_message(&json_str, &tool_call_tx)
                        }
                        Err(e) => {
                            error!("Failed to decode binary message: {}", e);
                            false
                        }
                    },
                    Message::Close(frame) => {
                        if let Some(close_frame) = frame {
                            warn!(
                                "WebSocket closed by server: code={}, reason={}",
                                close_frame.code, close_frame.reason
                            );
                        } else {
                            info!("WebSocket closed by server");
                        }
                        true
                    }
                    Message::Ping(data) => {
                        let mut stream_guard = ws_stream.write().await;
                        if let Some(stream) = stream_guard.as_mut() {
                            let _ = stream.send(Message::Pong(data)).await;
                        }
                        false
                    }
                    _ => false,
                };

                if should_break {
                    connected.store(false, Ordering::Relaxed);
                    break;
                }
            }
        });
    }

    fn process_json_message(
        json_str: &str,
        tool_call_tx: &mpsc::UnboundedSender<Vec<FunctionCall>>,
    ) -> bool {
        match serde_json::from_str::<Value>(json_str) {
            Ok(v) => {
                if v.get("setupComplete").is_some() {
                    info!("Setup complete");
                } else if let Some(sc) = v.get("serverContent") {
                    match serde_json::from_value::<crate::types::ServerContent>(sc.clone()) {
                        Ok(content) => {
                            if let Some(text) = content.extract_text() {
                                info!("Model response: {}", text);
                            } else {
                                info!("Server content (no text): {}", sc);
                            }
                        }
                        Err(_) => {
                            info!("Server content: {}", sc);
                        }
                    }
                } else if let Some(tc) = v.get("toolCall") {
                    match serde_json::from_value::<ToolCall>(tc.clone()) {
                        Ok(tool_call) => {
                            if !tool_call.function_calls.is_empty()
                                && let Err(e) = tool_call_tx.send(tool_call.function_calls)
                            {
                                error!("Failed to send tool call: {}", e);
                            }
                        }
                        Err(e) => {
                            error!("Failed to parse toolCall: {}", e);
                        }
                    }
                } else if let Some(go) = v.get("goAway") {
                    let reason = go
                        .get("reason")
                        .and_then(|r| r.as_str())
                        .unwrap_or("unknown");
                    warn!("Server requested disconnect: {}", reason);
                    return true;
                } else {
                    debug!("Other message: {}", v);
                }
            }
            Err(e) => {
                warn!("Failed to parse message: {}. Raw: {}", e, json_str);
            }
        }
        false
    }

    pub async fn send_prompt(&self, text: impl Into<String>) -> Result<()> {
        let content = ClientContent::from_text(text);
        self.send_message(ClientMessage::ClientContent {
            client_content: content,
        })
        .await
    }

    pub async fn send_image_prompt(
        &self,
        text: impl Into<String>,
        image_base64: String,
        mime_type: String,
    ) -> Result<()> {
        let content = ClientContent::from_text_and_image(text, image_base64, mime_type);
        self.send_message(ClientMessage::ClientContent {
            client_content: content,
        })
        .await
    }

    pub async fn analyze_screenshot(&self, image_base64: String) -> Result<()> {
        let prompt = "Analyze this screenshot and describe the UI components, layout, and any visible functionality. Be detailed and specific.";
        self.send_image_prompt(prompt, image_base64, "image/png".to_string())
            .await
    }

    pub async fn extract_ui_components(&self, image_base64: String) -> Result<()> {
        let prompt = "Extract all UI components from this screenshot. For each component, identify: type (button, input, card, etc.), position, styling, and any text content. Format as JSON.";
        self.send_image_prompt(prompt, image_base64, "image/png".to_string())
            .await
    }

    pub async fn screenshot_to_code(&self, image_base64: String, framework: &str) -> Result<()> {
        let prompt = format!(
            "Convert this screenshot into {framework} code. Generate clean, production-ready code that recreates this UI. Include all components, styling, and layout."
        );
        self.send_image_prompt(prompt, image_base64, "image/png".to_string())
            .await
    }

    pub async fn send_tool_response(
        &self,
        id: impl Into<String>,
        response: serde_json::Value,
    ) -> Result<()> {
        let response_msg = ToolResponse::new(id, response);
        self.send_message(ClientMessage::ToolResponse {
            tool_response: response_msg,
        })
        .await
    }

    pub async fn receive_tool_calls(&self) -> Result<Vec<FunctionCall>> {
        let mut rx_guard = self.tool_call_rx.write().await;
        let rx = rx_guard.as_mut().ok_or(GeminiError::NotConnected)?;

        let result = match rx.recv().await {
            Some(calls) => Ok(calls),
            None => Err(GeminiError::SessionClosed(
                "Tool call channel closed".to_string(),
            )),
        };
        drop(rx_guard);
        result
    }

    pub async fn close(&self) -> Result<()> {
        self.connected.store(false, Ordering::Relaxed);
        let mut stream_guard = self.ws_stream.write().await;
        if let Some(mut stream) = stream_guard.take() {
            let result = stream.send(Message::Close(None)).await;
            drop(stream_guard);
            result?;
        }
        Ok(())
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }
}
