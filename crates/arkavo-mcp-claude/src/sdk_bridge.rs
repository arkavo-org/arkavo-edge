use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt;
use tracing::{debug, error, info};

use anthropic_agent_sdk::types::introspection::ModelUsage;
use anthropic_agent_sdk::{ClaudeAgentOptions, ClaudeSDKClient, Message, auth::OAuthClient, query};

use crate::config::ClaudeCodeConfig;
use crate::event_mapper::EventMapper;
use crate::hook_handler;
use crate::policy_bridge::PolicyBridge;
use crate::{ClaudeCodeError, Result};

/// Authentication method for Claude API
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// OAuth for Claude Max/Pro subscribers (no API key needed)
    OAuth,
    /// API key authentication
    ApiKey(String),
}

impl Default for AuthMethod {
    fn default() -> Self {
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY")
            && !key.is_empty()
        {
            return AuthMethod::ApiKey(key);
        }
        AuthMethod::OAuth
    }
}

/// Active session mode
enum SessionMode {
    /// Bidirectional client for stateful sessions
    Client(ClaudeSDKClient),
}

/// Bridge to the native Rust Claude Agent SDK.
///
/// Supports both `ClaudeSDKClient` (bidirectional) and `query()` (one-shot).
/// Budget metrics from `Message::Result` are accumulated for AG-UI reporting.
pub struct SdkBridge {
    event_mapper: Arc<EventMapper>,
    auth_method: AuthMethod,
    options: ClaudeAgentOptions,
    session: tokio::sync::RwLock<Option<SessionMode>>,
    config: ClaudeCodeConfig,
    /// Accumulated cost in USD (stored as u64 bits of f64)
    accumulated_cost_bits: AtomicU64,
    /// Accumulated token counts
    accumulated_input_tokens: AtomicU64,
    accumulated_output_tokens: AtomicU64,
}

impl SdkBridge {
    /// Create a new SDK bridge with policy enforcement
    pub fn new(
        config: &ClaudeCodeConfig,
        event_mapper: Arc<EventMapper>,
        policy_bridge: Option<Arc<PolicyBridge>>,
    ) -> Result<Self> {
        let auth_method = if let Some(token) = &config.anthropic_auth_token {
            AuthMethod::ApiKey(token.clone())
        } else if config.use_oauth {
            AuthMethod::OAuth
        } else {
            AuthMethod::default()
        };

        // Set API key in environment early
        if let AuthMethod::ApiKey(key) = &auth_method {
            unsafe {
                std::env::set_var("ANTHROPIC_API_KEY", key);
            }
        }

        // Allow launching Claude Code from within an existing session.
        // The CLAUDECODE env var triggers a nested-session check in the CLI
        // that blocks subprocess launches. Since we're an orchestrator (not a
        // recursive invocation), removing it is safe.
        unsafe {
            std::env::remove_var("CLAUDECODE");
        }

        // Build options from config, then overlay hooks and permissions
        let mut options = config.build_agent_options();

        // Wire hooks for AG-UI event emission
        let hooks = hook_handler::build_hooks(event_mapper.clone());
        options.hooks = Some(hooks);

        // Wire permission callback if policy bridge is provided
        if let Some(bridge) = policy_bridge {
            let callback = hook_handler::build_permission_callback(bridge, event_mapper.clone());
            options.can_use_tool = Some(callback);
        }

        Ok(Self {
            event_mapper,
            auth_method,
            options,
            session: tokio::sync::RwLock::new(None),
            config: config.clone(),
            accumulated_cost_bits: AtomicU64::new(0),
            accumulated_input_tokens: AtomicU64::new(0),
            accumulated_output_tokens: AtomicU64::new(0),
        })
    }

    /// Initialize authentication (OAuth flow if needed)
    pub async fn initialize(&self) -> Result<()> {
        // Inside a Claude Code session the subprocess inherits the session
        // token — no explicit auth step is needed
        if std::env::var("CLAUDE_CODE_SESSION_ACCESS_TOKEN").is_ok() {
            info!("Initializing Claude SDK with inherited session token");
            return Ok(());
        }

        match &self.auth_method {
            AuthMethod::OAuth => {
                info!("Initializing Claude SDK with OAuth authentication");
                let oauth = OAuthClient::new().map_err(|e| {
                    ClaudeCodeError::Auth(format!("Failed to create OAuth client: {e}"))
                })?;

                if oauth.is_authenticated() {
                    info!("OAuth: Using cached authentication token");
                    return Ok(());
                }

                match oauth.authenticate().await {
                    Ok(token_info) => {
                        info!("OAuth: Authenticated successfully");
                        debug!("Token expires at: {:?}", token_info.expires_at);
                    }
                    Err(e) => {
                        return Err(ClaudeCodeError::Auth(format!(
                            "OAuth authentication required. Run 'claude login' or set ANTHROPIC_API_KEY.\nError: {e}"
                        )));
                    }
                }
            }
            AuthMethod::ApiKey(_) => {
                info!("Initializing Claude SDK with API key authentication");
            }
        }
        Ok(())
    }

    /// Start a bidirectional session with `ClaudeSDKClient`
    pub async fn start_session(&self) -> Result<()> {
        if !self.config.use_bidirectional {
            debug!("Bidirectional mode disabled, will use query() fallback");
            return Ok(());
        }

        let client = ClaudeSDKClient::new(self.options.clone(), None)
            .await
            .map_err(|e| ClaudeCodeError::Sdk(format!("Failed to create client: {e}")))?;

        info!("Bidirectional ClaudeSDKClient session started");
        *self.session.write().await = Some(SessionMode::Client(client));
        Ok(())
    }

    /// Run a query, using bidirectional client if available, else one-shot
    pub async fn run_query(&self, prompt: String, run_id: String) -> Result<String> {
        self.event_mapper.emit_run_started(&run_id, &prompt).await;

        let session = self.session.read().await;
        let result = if session.is_some() {
            drop(session);
            self.run_bidirectional(prompt, &run_id).await
        } else {
            drop(session);
            self.run_oneshot(prompt, &run_id).await
        };

        self.event_mapper.emit_run_completed(&run_id).await;
        result
    }

    /// Run via bidirectional `ClaudeSDKClient`
    #[allow(clippy::significant_drop_tightening)]
    async fn run_bidirectional(&self, prompt: String, run_id: &str) -> Result<String> {
        let mut session = self.session.write().await;
        let client = match session.as_mut() {
            Some(SessionMode::Client(c)) => c,
            None => return self.run_oneshot(prompt, run_id).await,
        };

        client
            .send_message(&prompt)
            .await
            .map_err(|e| ClaudeCodeError::Sdk(format!("Failed to send message: {e}")))?;

        let mut full_response = String::new();

        while let Some(result) = client.next_message().await {
            match result {
                Ok(message) => {
                    let done = matches!(&message, Message::Result { .. });
                    let text = self.handle_message(run_id, message).await;
                    full_response.push_str(&text);
                    if done {
                        break;
                    }
                }
                Err(e) => {
                    error!("Stream error: {e}");
                    self.event_mapper
                        .emit_error(run_id, &format!("Stream error: {e}"))
                        .await;
                    break;
                }
            }
        }

        Ok(full_response)
    }

    /// Run via one-shot `query()` API
    async fn run_oneshot(&self, prompt: String, run_id: &str) -> Result<String> {
        debug!("Using one-shot query() for run: {run_id}");
        let stream = query(&prompt, Some(self.options.clone()))
            .await
            .map_err(|e| ClaudeCodeError::Sdk(format!("Failed to start query: {e}")))?;

        let mut stream = Box::pin(stream);
        let mut full_response = String::new();

        while let Some(result) = stream.next().await {
            match result {
                Ok(message) => {
                    let text = self.handle_message(run_id, message).await;
                    full_response.push_str(&text);
                }
                Err(e) => {
                    error!("Stream error: {e}");
                    self.event_mapper
                        .emit_error(run_id, &format!("Stream error: {e}"))
                        .await;
                }
            }
        }

        Ok(full_response)
    }

    /// Handle a message from the SDK stream, return extracted text
    async fn handle_message(&self, run_id: &str, message: Message) -> String {
        match message {
            Message::Assistant { message, .. } => {
                let content = Self::extract_assistant_content(&message);
                self.event_mapper
                    .emit_assistant_message(run_id, &content)
                    .await;
                content
            }
            Message::Result {
                total_cost_usd,
                model_usage,
                duration_ms,
                num_turns,
                permission_denials,
                result,
                ..
            } => {
                self.accumulate_usage(total_cost_usd, &model_usage);
                self.event_mapper
                    .emit_run_metrics(run_id, duration_ms, num_turns, total_cost_usd)
                    .await;
                for denial in &permission_denials {
                    self.event_mapper
                        .emit_permission_decision(
                            run_id,
                            &denial.tool_name,
                            false,
                            "denied by Claude Code",
                        )
                        .await;
                }
                result.unwrap_or_default()
            }
            Message::System { subtype, .. } => {
                debug!("System message: {subtype}");
                String::new()
            }
            _ => String::new(),
        }
    }

    /// Accumulate usage metrics from a Result message
    fn accumulate_usage(
        &self,
        total_cost_usd: Option<f64>,
        model_usage: &HashMap<String, ModelUsage>,
    ) {
        if let Some(cost) = total_cost_usd {
            let prev_bits = self.accumulated_cost_bits.load(Ordering::Relaxed);
            let prev = f64::from_bits(prev_bits);
            let new = prev + cost;
            self.accumulated_cost_bits
                .store(new.to_bits(), Ordering::Relaxed);
        }
        for usage in model_usage.values() {
            self.accumulated_input_tokens
                .fetch_add(usage.input_tokens, Ordering::Relaxed);
            self.accumulated_output_tokens
                .fetch_add(usage.output_tokens, Ordering::Relaxed);
        }
    }

    /// Get accumulated cost in USD
    pub fn accumulated_cost_usd(&self) -> f64 {
        f64::from_bits(self.accumulated_cost_bits.load(Ordering::Relaxed))
    }

    /// Get accumulated input tokens
    pub fn accumulated_input_tokens(&self) -> u64 {
        self.accumulated_input_tokens.load(Ordering::Relaxed)
    }

    /// Get accumulated output tokens
    pub fn accumulated_output_tokens(&self) -> u64 {
        self.accumulated_output_tokens.load(Ordering::Relaxed)
    }

    /// Interrupt the current run (bidirectional only)
    #[allow(clippy::significant_drop_tightening)]
    pub async fn interrupt(&self) -> Result<()> {
        let mut session = self.session.write().await;
        if let Some(SessionMode::Client(client)) = session.as_mut() {
            client
                .interrupt()
                .await
                .map_err(|e| ClaudeCodeError::Sdk(format!("Interrupt failed: {e}")))?;
        }
        Ok(())
    }

    /// Get session info (bidirectional only)
    pub async fn session_info(&self) -> Result<serde_json::Value> {
        let session = self.session.read().await;
        match session.as_ref() {
            Some(SessionMode::Client(client)) => {
                let info = client.session_info();
                serde_json::to_value(info)
                    .map_err(|e| ClaudeCodeError::Other(format!("Serialize error: {e}")))
            }
            None => Ok(serde_json::json!({"status": "no active session"})),
        }
    }

    /// Close the session
    pub async fn close_session(&self) -> Result<()> {
        let session = self.session.write().await.take();
        if let Some(SessionMode::Client(mut client)) = session
            && let Err(e) = client.close().await
        {
            tracing::warn!("Error closing session: {e}");
        }
        info!("Session resources released");
        Ok(())
    }

    /// Check if OAuth is already authenticated
    pub fn is_authenticated(&self) -> bool {
        if matches!(&self.auth_method, AuthMethod::OAuth)
            && let Ok(oauth) = OAuthClient::new()
        {
            return oauth.is_authenticated();
        }
        matches!(&self.auth_method, AuthMethod::ApiKey(_))
    }

    /// Get the current authentication method
    pub fn auth_method(&self) -> &AuthMethod {
        &self.auth_method
    }

    /// Extract text content from an assistant message
    fn extract_assistant_content(
        message: &anthropic_agent_sdk::types::AssistantMessageContent,
    ) -> String {
        use anthropic_agent_sdk::types::ContentBlock;

        let mut parts = Vec::new();
        for block in &message.content {
            match block {
                ContentBlock::Text { text } => parts.push(text.clone()),
                ContentBlock::ToolUse { name, input, .. } => {
                    parts.push(format!("[Tool: {name} with input: {input}]"));
                }
                _ => {}
            }
        }
        parts.join("\n")
    }
}
