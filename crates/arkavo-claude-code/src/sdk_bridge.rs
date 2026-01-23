use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

use anthropic_agent_sdk::{auth::OAuthClient, query, ClaudeAgentOptions, Message};

use crate::event_mapper::EventMapper;
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
        // Check for API key first, fall back to OAuth
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            if !key.is_empty() {
                return AuthMethod::ApiKey(key);
            }
        }
        AuthMethod::OAuth
    }
}

/// Bridge to the native Rust Claude Agent SDK
pub struct SdkBridge {
    event_mapper: Arc<EventMapper>,
    auth_method: AuthMethod,
    options: ClaudeAgentOptions,
    _authenticated: Arc<RwLock<bool>>,
}

impl SdkBridge {
    /// Create a new SDK bridge
    pub async fn new(
        config: &crate::config::ClaudeCodeConfig,
        event_mapper: Arc<EventMapper>,
    ) -> Result<Self> {
        let auth_method = if let Some(token) = &config.anthropic_auth_token {
            AuthMethod::ApiKey(token.clone())
        } else if config.use_oauth {
            AuthMethod::OAuth
        } else {
            AuthMethod::default()
        };

        // Build options for the Claude agent
        let options = ClaudeAgentOptions::builder()
            .cwd(config.workspace_root.clone())
            .build();

        Ok(Self {
            event_mapper,
            auth_method,
            options,
            _authenticated: Arc::new(RwLock::new(false)),
        })
    }

    /// Initialize authentication (OAuth flow if needed)
    pub async fn initialize(&self) -> Result<()> {
        match &self.auth_method {
            AuthMethod::OAuth => {
                info!("Initializing Claude SDK with OAuth authentication");

                // Create OAuth client
                let oauth = OAuthClient::new().map_err(|e| {
                    ClaudeCodeError::Auth(format!("Failed to create OAuth client: {}", e))
                })?;

                // Check if we already have a valid cached token
                if oauth.is_authenticated() {
                    info!("OAuth: Using cached authentication token");
                    return Ok(());
                }

                // No cached token - need to authenticate
                // In non-interactive contexts, provide instructions
                info!("OAuth: No cached token found, authentication required");

                // Try to authenticate - this will open browser and prompt for code
                // In agent contexts, this may fail if stdin is not available
                match oauth.authenticate().await {
                    Ok(token_info) => {
                        info!("OAuth: Authenticated successfully");
                        debug!("Token expires at: {:?}", token_info.expires_at);
                    }
                    Err(e) => {
                        // Provide helpful error message for non-interactive contexts
                        return Err(ClaudeCodeError::Auth(format!(
                            "OAuth authentication required.\n\
                             \n\
                             To authenticate, run one of these commands in a terminal:\n\
                             \n\
                             Option 1 - Claude CLI (if installed):\n\
                             $ claude login\n\
                             \n\
                             Option 2 - Direct OAuth (opens browser):\n\
                             The OAuth flow opened a browser. Paste the authorization code when prompted.\n\
                             \n\
                             Option 3 - Use API key instead:\n\
                             $ export ANTHROPIC_API_KEY=\"sk-ant-...\"\n\
                             \n\
                             Original error: {}",
                            e
                        )));
                    }
                }
            }
            AuthMethod::ApiKey(key) => {
                info!("Initializing Claude SDK with API key authentication");
                // SAFETY: We're setting a single env var before any threads use it
                unsafe {
                    std::env::set_var("ANTHROPIC_API_KEY", key);
                }
            }
        }

        info!("Claude SDK initialized successfully");
        Ok(())
    }

    /// Check if OAuth is already authenticated (has cached token)
    pub fn is_authenticated(&self) -> bool {
        if let AuthMethod::OAuth = &self.auth_method {
            if let Ok(oauth) = OAuthClient::new() {
                return oauth.is_authenticated();
            }
        }
        // API key auth is always "authenticated" if key is set
        matches!(&self.auth_method, AuthMethod::ApiKey(_))
    }

    /// Run a simple query and stream results
    pub async fn run_query(&self, prompt: String, run_id: String) -> Result<()> {
        debug!("Starting query run: {}", run_id);

        // Emit start event
        self.event_mapper.emit_run_started(&run_id, &prompt).await;

        // Use the simple query API for one-shot interactions
        let stream = query(&prompt, Some(self.options.clone()))
            .await
            .map_err(|e| ClaudeCodeError::Sdk(format!("Failed to start query: {}", e)))?;

        let mut stream = Box::pin(stream);
        let event_mapper = self.event_mapper.clone();
        let run_id_clone = run_id.clone();

        // Process the message stream
        while let Some(result) = stream.next().await {
            match result {
                Ok(message) => {
                    self.handle_message(&run_id_clone, message, &event_mapper)
                        .await;
                }
                Err(e) => {
                    error!("Stream error: {}", e);
                    event_mapper
                        .emit_error(&run_id_clone, &format!("Stream error: {}", e))
                        .await;
                }
            }
        }

        // Emit completion event
        self.event_mapper.emit_run_completed(&run_id).await;

        debug!("Query run completed: {}", run_id);
        Ok(())
    }

    /// Close any session resources
    pub async fn close_session(&self) -> Result<()> {
        info!("Session resources released");
        Ok(())
    }

    /// Handle a message from the SDK stream
    async fn handle_message(&self, run_id: &str, message: Message, event_mapper: &EventMapper) {
        match message {
            Message::Assistant { message, .. } => {
                debug!("Assistant message received");
                let content = format!("{:?}", message);
                event_mapper.emit_assistant_message(run_id, &content).await;
            }
            Message::User { message, .. } => {
                debug!("User message echoed: {:?}", message);
            }
            Message::Result { result, .. } => {
                debug!("Result: {:?}", result);
                if let Some(res) = result {
                    event_mapper.emit_result(run_id, &format!("{:?}", res)).await;
                }
            }
            _ => {
                debug!("Other message type received");
            }
        }
    }

    /// Get the current authentication method
    pub fn auth_method(&self) -> &AuthMethod {
        &self.auth_method
    }
}
