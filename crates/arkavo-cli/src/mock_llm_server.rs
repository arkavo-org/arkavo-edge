//! Mock LLM HTTP Server for E2E Security Testing
//!
//! This module provides a mock HTTP server that mimics LLM provider APIs
//! (OpenAI, Anthropic, etc.) for end-to-end security testing.
//!
//! When ARKAVO_MOCK_LLM_SERVER=1 is set, HTTP requests to LLM APIs
//! are intercepted and routed to this mock server instead.
//!
//! Features:
//! - PII/DLP detection in requests
//! - Simulated API responses
//! - Request/response logging for verification
//! - Configurable blocking behavior

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

/// Detection flags for security features
#[derive(Debug, Clone, Copy)]
pub struct DetectionFlags {
    /// Whether to detect PII in requests
    pub detect_pii: bool,
    /// Whether to detect DLP violations
    pub detect_dlp: bool,
    /// Whether to block requests with sensitive data
    pub block_sensitive_requests: bool,
}

impl Default for DetectionFlags {
    fn default() -> Self {
        Self {
            detect_pii: true,
            detect_dlp: true,
            block_sensitive_requests: true,
        }
    }
}

/// Mock LLM server configuration
#[derive(Debug, Clone)]
pub struct MockLlmServerConfig {
    /// Port to listen on
    pub port: u16,
    /// Detection flags for security features
    pub detection: DetectionFlags,
    /// Simulated API key validation
    pub validate_api_key: bool,
    /// Expected API key
    pub expected_api_key: String,
}

impl Default for MockLlmServerConfig {
    fn default() -> Self {
        Self {
            port: 9999,
            detection: DetectionFlags::default(),
            validate_api_key: true,
            expected_api_key: "test-api-key".to_string(),
        }
    }
}

/// Recorded request for verification
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub timestamp: std::time::SystemTime,
    pub provider: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: String,
    pub blocked: bool,
    pub block_reason: Option<String>,
}

/// Recorded response for verification
#[derive(Debug, Clone)]
pub struct RecordedResponse {
    pub timestamp: std::time::SystemTime,
    pub status_code: u16,
    pub body: String,
}

/// Mock LLM Server state
pub struct MockLlmServer {
    config: MockLlmServerConfig,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    responses: Arc<Mutex<Vec<RecordedResponse>>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Default for MockLlmServer {
    fn default() -> Self {
        Self::new()
    }
}

impl MockLlmServer {
    /// Create a new mock server with default config
    pub fn new() -> Self {
        Self::with_config(MockLlmServerConfig::default())
    }

    /// Create a new mock server with custom config
    pub fn with_config(config: MockLlmServerConfig) -> Self {
        Self {
            config,
            requests: Arc::new(Mutex::new(Vec::new())),
            responses: Arc::new(Mutex::new(Vec::new())),
            shutdown_tx: None,
        }
    }

    /// Check if mock server should intercept requests
    pub fn is_enabled() -> bool {
        std::env::var("ARKAVO_MOCK_LLM_SERVER")
            .map(|v| v == "1" || v == "true")
            .unwrap_or(false)
    }

    /// Get the proxy URL for HTTP clients to use
    pub fn proxy_url() -> String {
        format!("http://127.0.0.1:{}", Self::default_port())
    }

    /// Default port for mock server
    pub fn default_port() -> u16 {
        9999
    }

    /// Start the mock server
    pub async fn start(&mut self) -> Result<SocketAddr, Box<dyn std::error::Error>> {
        let addr = SocketAddr::from(([127, 0, 0, 1], self.config.port));

        // Build router
        let app = self.build_router();

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        // Start server with graceful shutdown
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let actual_addr = listener.local_addr()?;

        let _server = axum::serve(listener, app);

        tokio::spawn(async move {
            let _ = shutdown_rx.await;
        });

        println!("🔒 Mock LLM Server started on {actual_addr}");
        println!(
            "   Set https_proxy={} to intercept requests",
            Self::proxy_url()
        );

        Ok(actual_addr)
    }

    /// Build the HTTP router
    fn build_router(&self) -> axum::Router {
        use axum::routing::{get, post};

        let requests = self.requests.clone();
        let responses = self.responses.clone();
        let config = self.config.clone();

        axum::Router::new()
            // OpenAI-compatible endpoints
            .route("/v1/chat/completions", post(handle_chat_completion))
            .route("/v1/completions", post(handle_completion))
            .route("/v1/models", get(handle_models))
            // Anthropic-compatible endpoints
            .route("/v1/messages", post(handle_anthropic_messages))
            // Gemini-compatible endpoints
            .route(
                "/v1beta/models/{model}:generateContent",
                post(handle_gemini_generate),
            )
            // Health check
            .route("/health", get(|| async { "OK" }))
            .layer(axum::Extension(ServerState {
                config,
                requests,
                responses,
            }))
    }

    /// Stop the server
    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }

    /// Get recorded requests
    ///
    /// # Panics
    /// Panics if the internal request mutex is poisoned
    pub fn get_requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Get recorded responses
    ///
    /// # Panics
    /// Panics if the internal response mutex is poisoned
    pub fn get_responses(&self) -> Vec<RecordedResponse> {
        self.responses.lock().unwrap().clone()
    }

    /// Clear logs
    ///
    /// # Panics
    /// Panics if the internal request or response mutex is poisoned
    pub fn clear_logs(&self) {
        self.requests.lock().unwrap().clear();
        self.responses.lock().unwrap().clear();
    }

    /// Check if sensitive data was detected in any request
    ///
    /// # Panics
    /// Panics if the internal request mutex is poisoned
    pub fn has_sensitive_data(&self) -> bool {
        self.requests.lock().unwrap().iter().any(|r| r.blocked)
    }
}

/// Server state shared across handlers
#[derive(Clone)]
struct ServerState {
    config: MockLlmServerConfig,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    responses: Arc<Mutex<Vec<RecordedResponse>>>,
}

/// Handle OpenAI-style chat completion
async fn handle_chat_completion(
    axum::Extension(state): axum::Extension<ServerState>,
    headers: axum::http::HeaderMap,
    body: String,
) -> axum::response::Response<String> {
    // Extract Authorization header
    let auth_header = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    // Validate API key if configured
    if state.config.validate_api_key {
        let expected = format!("Bearer {}", state.config.expected_api_key);
        if auth_header != expected {
            return axum::response::Response::builder()
                .status(401)
                .body(
                    json!({
                        "error": {
                            "message": "Invalid API key",
                            "type": "authentication_error"
                        }
                    })
                    .to_string(),
                )
                .unwrap();
        }
    }

    // Check for PII/DLP in request body
    let (blocked, block_reason) =
        if state.config.detection.detect_pii || state.config.detection.detect_dlp {
            check_sensitive_data(&body, &state.config)
        } else {
            (false, None)
        };

    // Record the request
    let request = RecordedRequest {
        timestamp: std::time::SystemTime::now(),
        provider: "openai".to_string(),
        path: "/v1/chat/completions".to_string(),
        headers: extract_headers(&headers),
        body: body.clone(),
        blocked,
        block_reason: block_reason.clone(),
    };
    state.requests.lock().unwrap().push(request);

    // If blocked, return error
    if blocked && state.config.detection.block_sensitive_requests {
        let response_body = json!({
            "error": {
                "message": block_reason.unwrap_or_else(|| "Request blocked by security policy".to_string()),
                "type": "security_policy_violation",
                "code": "dlp_violation"
            }
        }).to_string();

        state.responses.lock().unwrap().push(RecordedResponse {
            timestamp: std::time::SystemTime::now(),
            status_code: 400,
            body: response_body.clone(),
        });

        return axum::response::Response::builder()
            .status(400)
            .header("content-type", "application/json")
            .body(response_body)
            .unwrap();
    }

    // Generate mock response
    let response_body = generate_openai_response(&body);

    state.responses.lock().unwrap().push(RecordedResponse {
        timestamp: std::time::SystemTime::now(),
        status_code: 200,
        body: response_body.clone(),
    });

    axum::response::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(response_body)
        .unwrap()
}

/// Handle completion endpoint
#[allow(unused_variables)]
async fn handle_completion(
    axum::Extension(state): axum::Extension<ServerState>,
    headers: axum::http::HeaderMap,
    body: String,
) -> axum::response::Response<String> {
    // Similar to chat completion but simpler response format
    let response_body = json!({
        "id": "mock-completion",
        "object": "text_completion",
        "created": chrono::Utc::now().timestamp(),
        "model": "mock-model",
        "choices": [{
            "text": "This is a mock completion response for testing.",
            "index": 0,
            "logprobs": null,
            "finish_reason": "stop"
        }]
    })
    .to_string();

    axum::response::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(response_body)
        .unwrap()
}

/// Handle models endpoint
async fn handle_models() -> axum::response::Response<String> {
    let response_body = json!({
        "object": "list",
        "data": [
            {
                "id": "mock-gpt-4",
                "object": "model",
                "created": chrono::Utc::now().timestamp(),
                "owned_by": "mock"
            },
            {
                "id": "mock-gpt-3.5-turbo",
                "object": "model",
                "created": chrono::Utc::now().timestamp(),
                "owned_by": "mock"
            }
        ]
    })
    .to_string();

    axum::response::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(response_body)
        .unwrap()
}

/// Handle Anthropic messages endpoint
#[allow(unused_variables)]
async fn handle_anthropic_messages(
    axum::Extension(state): axum::Extension<ServerState>,
    headers: axum::http::HeaderMap,
    body: String,
) -> axum::response::Response<String> {
    // Check for PII/DLP
    let (blocked, block_reason) = check_sensitive_data(&body, &state.config);

    if blocked && state.config.detection.block_sensitive_requests {
        return axum::response::Response::builder()
            .status(400)
            .body(
                json!({
                    "type": "error",
                    "error": {
                        "type": "security_policy_violation",
                        "message": block_reason.unwrap_or_else(|| "Request blocked".to_string())
                    }
                })
                .to_string(),
            )
            .unwrap();
    }

    let response_body = json!({
        "id": format!("mock-msg-{}", uuid::Uuid::new_v4()),
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "Mock Anthropic response for testing."}],
        "model": "claude-3-mock",
        "stop_reason": "end_turn"
    })
    .to_string();

    axum::response::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(response_body)
        .unwrap()
}

/// Handle Gemini generate content endpoint
#[allow(unused_variables)]
async fn handle_gemini_generate(
    axum::Extension(state): axum::Extension<ServerState>,
    headers: axum::http::HeaderMap,
    body: String,
) -> axum::response::Response<String> {
    let response_body = json!({
        "candidates": [{
            "content": {
                "parts": [{"text": "Mock Gemini response for testing."}],
                "role": "model"
            },
            "finishReason": "STOP"
        }]
    })
    .to_string();

    axum::response::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(response_body)
        .unwrap()
}

/// Check for sensitive data in request body
fn check_sensitive_data(body: &str, config: &MockLlmServerConfig) -> (bool, Option<String>) {
    let mut findings = Vec::new();

    // Check for PII patterns
    if config.detection.detect_pii {
        // Email
        if let Ok(re) = regex::Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}")
            && re.is_match(body)
        {
            findings.push("email address");
        }
        // Phone
        if let Ok(re) = regex::Regex::new(r"\b\d{3}[-.]?\d{3}[-.]?\d{4}\b")
            && re.is_match(body)
        {
            findings.push("phone number");
        }
        // SSN
        if let Ok(re) = regex::Regex::new(r"\b\d{3}-\d{2}-\d{4}\b")
            && re.is_match(body)
        {
            findings.push("SSN");
        }
        // API keys
        if let Ok(re) = regex::Regex::new(r"\b(?:sk-|pk-)[a-zA-Z0-9]{16,}\b")
            && re.is_match(body)
        {
            findings.push("API key");
        }
    }

    // Check for DLP patterns
    if config.detection.detect_dlp {
        // AWS credentials
        if body.to_lowercase().contains("aws_secret_access_key") {
            findings.push("AWS credentials");
        }
        // Private keys
        if body.contains("PRIVATE KEY") {
            findings.push("private key");
        }
        // Internal IPs
        if let Ok(re) = regex::Regex::new(r"\b(?:10\.\d{1,3}|192\.168)\.\d{1,3}\.\d{1,3}\b")
            && re.is_match(body)
        {
            findings.push("internal IP");
        }
    }

    if findings.is_empty() {
        (false, None)
    } else {
        let reason = format!(
            "Request blocked: detected potential {}. Remove sensitive data before sending to LLM.",
            findings.join(", ")
        );
        (true, Some(reason))
    }
}

/// Generate OpenAI-style response
fn generate_openai_response(request_body: &str) -> String {
    // Extract the last user message for context
    let response_text = if request_body.contains("password") || request_body.contains("secret") {
        "I cannot help with passwords or secrets. This appears to be sensitive information."
    } else if request_body.contains("email") || request_body.contains('@') {
        "I notice you may have included an email address. For privacy, avoid sharing personal information."
    } else {
        "This is a mock response from the Arkavo security testing server. Your request was processed safely."
    };

    json!({
        "id": format!("mock-chatcmpl-{}", uuid::Uuid::new_v4().to_string()),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": "mock-gpt-4",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": response_text
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 20,
            "total_tokens": 30
        }
    })
    .to_string()
}

/// Extract relevant headers
fn extract_headers(headers: &axum::http::HeaderMap) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for (key, value) in headers.iter() {
        if let Ok(value_str) = value.to_str() {
            // Only capture non-sensitive headers
            if key.as_str() != "authorization" {
                result.insert(key.to_string(), value_str.to_string());
            } else {
                result.insert(key.to_string(), "[REDACTED]".to_string());
            }
        }
    }
    result
}

use serde_json::json;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_server_starts() {
        let mut server = MockLlmServer::new();
        let addr = server.start().await.unwrap();
        assert!(addr.port() > 0);
        server.stop();
    }

    #[test]
    fn test_pii_detection() {
        let config = MockLlmServerConfig::default();

        let (blocked, reason) = check_sensitive_data("Contact me at user@example.com", &config);
        assert!(blocked);
        assert!(reason.unwrap().contains("email"));
    }

    #[test]
    fn test_dlp_detection() {
        let config = MockLlmServerConfig::default();

        let (blocked, reason) =
            check_sensitive_data("AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG", &config);
        assert!(blocked);
        assert!(reason.unwrap().contains("AWS"));
    }

    #[test]
    fn test_safe_content_not_blocked() {
        let config = MockLlmServerConfig::default();

        let (blocked, reason) = check_sensitive_data("What is the weather today?", &config);
        assert!(!blocked);
        assert!(reason.is_none());
    }

    #[test]
    fn test_detection_flags_default() {
        let flags = DetectionFlags::default();
        assert!(flags.detect_pii);
        assert!(flags.detect_dlp);
        assert!(flags.block_sensitive_requests);
    }
}
