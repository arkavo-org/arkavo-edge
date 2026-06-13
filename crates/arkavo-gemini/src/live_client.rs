use crate::error::{GeminiError, Result};
use crate::types::{
    ClientContent, ClientMessage, FunctionCall, FunctionDeclaration, GenerationConfig, SetupConfig,
    Tool, ToolCall, ToolResponse,
};
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tracing::{debug, error, info, warn};
use url::Url;

/// WebSocket endpoint for the Gemini Live (BidiGenerateContent) API.
///
/// **Not the default streaming surface.** Text streaming for normal chat /
/// agentic routing goes through `RestClient::stream_generate_content_*`
/// (HTTP SSE on `streamGenerateContent`), which is what `GeminiProvider`
/// and `arkavo-router` invoke in production.
///
/// `LiveSessionClient` and this WebSocket endpoint are for **bidi audio /
/// video sessions** — stateful sockets where the client streams microphone
/// frames or video and the model streams audio responses. The only models
/// currently exposed for `bidiGenerateContent` are the
/// `gemini-2.5-flash-native-audio-*` variants; the announced
/// `gemini-3.1-flash-live` is preview-gated.
///
/// Live API lives at `v1alpha` (REST sibling is `v1beta`).
pub const GEMINI_WS_ENDPOINT: &str = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1alpha.GenerativeService.BidiGenerateContent";

/// Default model identifier for Live (bidi) WebSocket sessions.
///
/// Set to the only model the public v1alpha endpoint actually routes for
/// `bidiGenerateContent` today. Override via `GEMINI_MODEL` /
/// `LiveSessionClient::new(...)` if your account has the 3.1-flash-live
/// preview enabled.
pub const DEFAULT_LIVE_MODEL: &str = "gemini-2.5-flash-native-audio-latest";

const MAX_RECONNECT_ATTEMPTS: u32 = 5;
const INITIAL_BACKOFF_MS: u64 = 1000;

/// Response modality requested in the Live API setup frame.
///
/// The currently-routable bidi models (`gemini-2.5-flash-native-audio-*`)
/// only emit `Audio`; `Text` is wired for the future `gemini-3.1-flash-live`
/// preview where text bidi is supported.
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
type WsWriter = SplitSink<WsStream, Message>;
type WsReader = SplitStream<WsStream>;
type SharedWriter = Arc<Mutex<Option<WsWriter>>>;

/// Stateful WebSocket session against the Gemini Live (BidiGenerateContent) API.
///
/// **Use this only for bidi audio / video sessions.** For text chat,
/// agentic tool loops, and structured JSON output, use `RestClient` — the
/// `streamGenerateContent` SSE endpoint is the production streaming surface
/// that `GeminiProvider` and `arkavo-router` invoke.
///
/// Typical Live API workflow: connect → stream microphone PCM frames →
/// receive model audio + function calls → send `ToolResponse` frames →
/// close.
///
/// The underlying WebSocket is split at connect-time so the receiver task
/// owns the read half and `send_message` / `close` contend only on the
/// writer mutex; this avoids the deadlock you get when both sides try to
/// take a single `RwLock<WebSocket>` while one of them is parked on
/// `stream.next().await`.
pub struct LiveSessionClient {
    api_key: String,
    model: String,
    tools: Vec<Value>,
    modality: LiveModality,
    endpoint_url: String,
    ws_writer: SharedWriter,
    tool_call_tx: mpsc::UnboundedSender<Vec<FunctionCall>>,
    tool_call_rx: Arc<RwLock<Option<mpsc::UnboundedReceiver<Vec<FunctionCall>>>>>,
    connected: Arc<AtomicBool>,
}

impl LiveSessionClient {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::new_with_tools(api_key, model, vec![])
    }

    /// Construct a Live (bidi) WebSocket client pinned to
    /// `DEFAULT_LIVE_MODEL`. For text streaming use `RestClient` — the
    /// production SSE surface — not this WebSocket path.
    pub fn new_default(api_key: impl Into<String>) -> Self {
        Self::new(api_key, DEFAULT_LIVE_MODEL)
    }

    /// Build a Live (bidi) client from `GEMINI_API_KEY` using the default
    /// Live-capable model. Not for text chat — see `RestClient`.
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
            endpoint_url: GEMINI_WS_ENDPOINT.to_string(),
            ws_writer: Arc::new(Mutex::new(None)),
            tool_call_tx: tx,
            tool_call_rx: Arc::new(RwLock::new(Some(rx))),
            connected: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Override the WebSocket endpoint URL. Intended for tests that use a
    /// local mock server; production callers should leave the default.
    #[doc(hidden)]
    pub fn with_endpoint_url(mut self, endpoint_url: impl Into<String>) -> Self {
        self.endpoint_url = endpoint_url.into();
        self
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
        let url_with_key = format!("{}?key={}", self.endpoint_url, self.api_key);
        let _url = Url::parse(&url_with_key)?;

        debug!("Connecting to Gemini WebSocket endpoint");
        let (ws_stream, _) = connect_async(&url_with_key).await?;
        let (writer, reader) = ws_stream.split();

        {
            let mut guard = self.ws_writer.lock().await;
            *guard = Some(writer);
        }

        self.send_setup().await?;
        self.connected.store(true, Ordering::Relaxed);

        self.start_receiver_task(reader);

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

        // Live API requires the fully-qualified `models/<id>` form in the
        // setup frame — server rejects the bare id with "not found".
        let model_id = if self.model.starts_with("models/") {
            self.model.clone()
        } else {
            format!("models/{}", self.model)
        };
        let setup = SetupConfig {
            model: model_id,
            generation_config: Some(GenerationConfig {
                response_modalities: vec![self.modality.as_str().to_string()],
                temperature: None,
                max_output_tokens: None,
            }),
            tools,
        };

        self.send_message(ClientMessage::Setup { setup }).await
    }

    async fn send_message(&self, message: ClientMessage) -> Result<()> {
        let json = serde_json::to_string(&message)?;
        info!("Sending message: {}", json);

        let mut guard = self.ws_writer.lock().await;
        let writer = guard.as_mut().ok_or(GeminiError::NotConnected)?;
        let send_result = writer.send(Message::Text(json)).await;
        drop(guard);
        send_result.map_err(GeminiError::from)
    }

    fn start_receiver_task(&self, reader: WsReader) {
        let tool_call_tx = self.tool_call_tx.clone();
        let connected = self.connected.clone();
        let ws_writer = self.ws_writer.clone();

        tokio::spawn(async move {
            info!("Receiver task started");
            let mut reader = reader;
            loop {
                let message = match reader.next().await {
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
                        // Reply via the shared writer mutex. Reader holds no
                        // writer lock during its own next() await, so this
                        // never deadlocks against close() or send_message().
                        {
                            let mut guard = ws_writer.lock().await;
                            if let Some(writer) = guard.as_mut() {
                                let _ = writer.send(Message::Pong(data)).await;
                            }
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

    /// Stream a base64-encoded audio chunk into the Live session.
    pub async fn send_audio(&self, audio_base64: String, mime_type: String) -> Result<()> {
        let content = ClientContent::from_audio(audio_base64, mime_type);
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
        // The receiver task owns the read half and never grabs the writer
        // mutex for long, so this lock is uncontended (no deadlock against
        // a parked reader). Taking the writer also closes the underlying
        // socket on drop, which makes reader.next() return None and the
        // receiver task exit cleanly.
        let taken = {
            let mut guard = self.ws_writer.lock().await;
            guard.take()
        };
        if let Some(mut writer) = taken {
            let _ = writer.send(Message::Close(None)).await;
            let _ = writer.close().await;
        }
        Ok(())
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }
}
