//! Read-only introspection and runtime setter methods for `ClaudeSDKClient`

use crate::error::{ClaudeError, Result};
use crate::types::{AccountInfo, ModelInfo, SessionInfo};

use super::super::ClaudeSDKClient;

impl ClaudeSDKClient {
    // ========================================================================
    // Introspection Methods
    // ========================================================================

    /// Get session information including model, tools, and MCP servers.
    ///
    /// Returns `None` if the init message has not been received yet.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use anthropic_agent_sdk::{ClaudeSDKClient, ClaudeAgentOptions};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let options = ClaudeAgentOptions::default();
    /// let client = ClaudeSDKClient::new(options, None).await?;
    ///
    /// // Wait for first message to ensure init is received
    /// if let Some(info) = client.session_info() {
    ///     println!("Model: {:?}", info.model);
    ///     println!("Available tools: {:?}", info.tool_names());
    ///     for server in &info.mcp_servers {
    ///         println!("MCP Server {}: status={}", server.name, server.status);
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn session_info(&self) -> Option<SessionInfo> {
        self.session_info.lock().ok()?.clone()
    }

    /// Get the current model being used.
    ///
    /// Convenience method that extracts the model from session info.
    /// Returns `None` if init has not been received or model is not set.
    #[must_use]
    pub fn current_model(&self) -> Option<String> {
        self.session_info().and_then(|info| info.model)
    }

    /// Get the list of available tools in this session.
    ///
    /// Returns an empty vector if init has not been received.
    #[must_use]
    pub fn available_tools(&self) -> Vec<crate::types::ToolInfo> {
        self.session_info()
            .map(|info| info.tools)
            .unwrap_or_default()
    }

    /// Get MCP server status for all configured servers.
    ///
    /// Returns the status of each MCP server including connection state
    /// and any errors. Returns an empty vector if no MCP servers are configured.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use anthropic_agent_sdk::{ClaudeSDKClient, ClaudeAgentOptions};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let options = ClaudeAgentOptions::default();
    /// let client = ClaudeSDKClient::new(options, None).await?;
    ///
    /// for server in client.mcp_server_status() {
    ///     if !server.is_connected() {
    ///         eprintln!("MCP server {} failed: {:?}", server.name, server.error);
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn mcp_server_status(&self) -> Vec<crate::types::McpServerStatus> {
        self.session_info()
            .map(|info| info.mcp_servers)
            .unwrap_or_default()
    }

    /// Get list of known Claude models.
    ///
    /// Returns a static list of known Claude models with their capabilities.
    /// Note: This is a static list and may not reflect all available models
    /// for your account. Use `current_model()` to see what's actually in use.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use anthropic_agent_sdk::{ClaudeSDKClient, ClaudeAgentOptions};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let options = ClaudeAgentOptions::default();
    /// let client = ClaudeSDKClient::new(options, None).await?;
    ///
    /// for model in ClaudeSDKClient::supported_models() {
    ///     println!("{}: {:?}", model.id, model.name);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn supported_models() -> Vec<ModelInfo> {
        ModelInfo::known_models()
    }

    /// Get available slash commands.
    ///
    /// Returns the list of slash commands available in this session.
    /// Commands may be defined in project configuration (`.claude/commands/`)
    /// or built into Claude Code.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use anthropic_agent_sdk::{ClaudeSDKClient, ClaudeAgentOptions};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let options = ClaudeAgentOptions::default();
    /// let client = ClaudeSDKClient::new(options, None).await?;
    ///
    /// for cmd in client.supported_commands() {
    ///     println!("/{} - {}", cmd.name, cmd.description);
    ///     if !cmd.argument_hint.is_empty() {
    ///         println!("  Usage: /{} {}", cmd.name, cmd.argument_hint);
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn supported_commands(&self) -> Vec<crate::types::SlashCommand> {
        // Commands are loaded from the init message's slash_commands field.
        // This includes both user-level (~/.claude/commands/) and
        // project-level (.claude/commands/) custom commands.
        //
        // Note: Requires setting_sources to include User and/or Project.
        self.session_info()
            .and_then(|info| {
                info.extra.get("slash_commands").and_then(|v| {
                    // The CLI sends slash_commands as an array of strings
                    if let Some(arr) = v.as_array() {
                        let commands: Vec<crate::types::SlashCommand> = arr
                            .iter()
                            .filter_map(|item| {
                                // Handle both string format and object format
                                if let Some(name) = item.as_str() {
                                    Some(crate::types::SlashCommand {
                                        name: name.to_string(),
                                        description: String::new(),
                                        argument_hint: String::new(),
                                    })
                                } else {
                                    // Try to deserialize as SlashCommand object
                                    serde_json::from_value(item.clone()).ok()
                                }
                            })
                            .collect();
                        Some(commands)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_default()
    }

    /// Get account information from OAuth credentials.
    ///
    /// Reads account information from the Claude credentials file.
    /// This is only available for OAuth-authenticated accounts (Max Plan),
    /// not for API key authentication.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Credentials file not found
    /// - Credentials file is invalid
    /// - Not using OAuth authentication
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use anthropic_agent_sdk::{ClaudeSDKClient, ClaudeAgentOptions};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let options = ClaudeAgentOptions::default();
    /// let client = ClaudeSDKClient::new(options, None).await?;
    ///
    /// match client.account_info() {
    ///     Ok(info) => {
    ///         println!("Email: {:?}", info.email);
    ///         println!("OAuth: {}", info.is_oauth);
    ///     }
    ///     Err(e) => println!("No account info: {}", e),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn account_info(&self) -> Result<AccountInfo> {
        // Account info is derived from the init message's apiKeySource field
        // and potentially other session data
        let session = self.session_info().ok_or_else(ClaudeError::not_connected)?;

        // Get apiKeySource from extra fields
        let api_key_source = session
            .extra
            .get("apiKeySource")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Determine if OAuth based on apiKeySource
        // "none" means OAuth/Max Plan, other values indicate API key sources
        let is_oauth = api_key_source.as_deref() == Some("none");

        Ok(AccountInfo {
            email: None,      // Not available in init message
            account_id: None, // Not available in init message
            is_oauth,
            organization_id: None, // Not available in init message
        })
    }

    // ========================================================================
    // Runtime Setters
    // ========================================================================

    /// Store a model preference locally.
    ///
    /// **NOTE:** Runtime model switching mid-session is NOT currently supported
    /// by the Claude CLI's stream-json protocol. This method only stores the value
    /// locally for SDK reference. To use a different model, start a new session:
    ///
    /// ```rust,no_run
    /// use anthropic_agent_sdk::{ClaudeAgentOptions, ClaudeSDKClient};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let options = ClaudeAgentOptions::builder()
    ///     .model("haiku")  // Set model at session start
    ///     .build();
    /// let mut client = ClaudeSDKClient::new(options, None).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_model(&self, model: impl Into<String>) {
        let model_str = model.into();
        tracing::debug!(model = %model_str, "set_model: storing locally (runtime switching not supported)");

        if let Ok(mut guard) = self.runtime_model.lock() {
            *guard = Some(model_str);
        }
    }

    /// Get the currently configured runtime model override.
    ///
    /// Returns `None` if no runtime override is set.
    #[must_use]
    pub fn get_runtime_model(&self) -> Option<String> {
        self.runtime_model.lock().ok()?.clone()
    }

    /// Store a permission mode preference locally.
    ///
    /// **NOTE:** Runtime permission mode switching mid-session is NOT currently
    /// supported by the Claude CLI's stream-json protocol. This method only stores
    /// the value locally for SDK reference. To set permission mode, use session options:
    ///
    /// ```rust,no_run
    /// use anthropic_agent_sdk::{ClaudeAgentOptions, ClaudeSDKClient, PermissionMode};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let options = ClaudeAgentOptions::builder()
    ///     .permission_mode(PermissionMode::AcceptEdits)  // Set at session start
    ///     .build();
    /// let mut client = ClaudeSDKClient::new(options, None).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Permission modes:
    /// - `Default`: Prompt for permission on sensitive operations
    /// - `AcceptEdits`: Auto-approve file edits
    /// - `Plan`: Plan-only mode, no execution
    /// - `BypassPermissions`: Auto-approve all operations (use with caution)
    pub fn set_permission_mode(&self, mode: crate::types::PermissionMode) {
        tracing::debug!(mode = ?mode, "set_permission_mode: storing locally (runtime switching not supported)");

        if let Ok(mut guard) = self.runtime_permission_mode.lock() {
            *guard = Some(mode);
        }
    }

    /// Get the currently configured runtime permission mode override.
    ///
    /// Returns `None` if no runtime override is set.
    #[must_use]
    pub fn get_runtime_permission_mode(&self) -> Option<crate::types::PermissionMode> {
        *self.runtime_permission_mode.lock().ok()?
    }

    /// Store a max thinking tokens preference locally.
    ///
    /// **NOTE:** Runtime thinking token adjustment mid-session is NOT currently
    /// supported by the Claude CLI's stream-json protocol. This method only stores
    /// the value locally for SDK reference. To set thinking tokens, use session options:
    ///
    /// ```rust,no_run
    /// use anthropic_agent_sdk::{ClaudeAgentOptions, ClaudeSDKClient};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let options = ClaudeAgentOptions::builder()
    ///     .max_thinking_tokens(20000)  // Set at session start
    ///     .build();
    /// let mut client = ClaudeSDKClient::new(options, None).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Extended thinking allows Claude to "think" before responding,
    /// improving quality for complex tasks.
    pub fn set_max_thinking_tokens(&self, tokens: u32) {
        tracing::debug!(
            tokens = tokens,
            "set_max_thinking_tokens: storing locally (runtime switching not supported)"
        );

        if let Ok(mut guard) = self.runtime_max_thinking_tokens.lock() {
            *guard = Some(tokens);
        }
    }

    /// Get the currently configured runtime max thinking tokens override.
    ///
    /// Returns `None` if no runtime override is set.
    #[must_use]
    pub fn get_runtime_max_thinking_tokens(&self) -> Option<u32> {
        *self.runtime_max_thinking_tokens.lock().ok()?
    }

    /// Clear all runtime overrides.
    ///
    /// This resets model, permission mode, and thinking tokens to their
    /// original values from the initial options.
    pub fn clear_runtime_overrides(&self) {
        if let Ok(mut guard) = self.runtime_model.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = self.runtime_permission_mode.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = self.runtime_max_thinking_tokens.lock() {
            *guard = None;
        }
    }
}
