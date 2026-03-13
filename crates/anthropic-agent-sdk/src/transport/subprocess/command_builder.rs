//! Command building logic for subprocess transport

use std::collections::HashMap;
use tokio::process::Command;

use crate::error::Result;
use crate::types::SystemPrompt;

use super::{ALLOWED_EXTRA_FLAGS, PromptInput, SubprocessTransport};

impl SubprocessTransport {
    /// Build CLI command with all arguments
    #[allow(clippy::too_many_lines)]
    pub(super) fn build_command(&self) -> Result<Command> {
        let mut cmd = Command::new(&self.cli_path);

        // Always use --print for non-interactive mode to avoid terminal manipulation
        cmd.arg("--print");

        cmd.arg("--output-format")
            .arg("stream-json")
            .arg("--verbose");

        // System prompt
        if let Some(ref system_prompt) = self.options.system_prompt {
            match system_prompt {
                SystemPrompt::String(s) => {
                    cmd.arg("--system-prompt").arg(s);
                }
                SystemPrompt::Preset(preset) => {
                    if let Some(ref append) = preset.append {
                        cmd.arg("--append-system-prompt").arg(append);
                    }
                }
                SystemPrompt::File(path) => {
                    cmd.arg("--system-prompt-file").arg(path);
                }
            }
        }

        // Append system prompt (additional text to append to any system prompt)
        if let Some(ref append) = self.options.append_system_prompt {
            cmd.arg("--append-system-prompt").arg(append);
        }

        // Allowed tools
        if !self.options.allowed_tools.is_empty() {
            let tools: Vec<String> = self
                .options
                .allowed_tools
                .iter()
                .map(|t| t.as_str().to_string())
                .collect();
            cmd.arg("--allowedTools").arg(tools.join(","));
        }

        // Max turns
        if let Some(max_turns) = self.options.max_turns {
            cmd.arg("--max-turns").arg(max_turns.to_string());
        }

        // Disallowed tools
        if !self.options.disallowed_tools.is_empty() {
            let tools: Vec<String> = self
                .options
                .disallowed_tools
                .iter()
                .map(|t| t.as_str().to_string())
                .collect();
            cmd.arg("--disallowedTools").arg(tools.join(","));
        }

        // Model
        if let Some(ref model) = self.options.model {
            cmd.arg("--model").arg(model);
        }

        // Permission prompt tool
        if let Some(ref tool) = self.options.permission_prompt_tool_name {
            cmd.arg("--permission-prompt-tool").arg(tool);
        }

        // Permission mode
        if let Some(ref mode) = self.options.permission_mode {
            let mode_str = match mode {
                crate::types::PermissionMode::Default => "default",
                crate::types::PermissionMode::AcceptEdits => "acceptEdits",
                crate::types::PermissionMode::Plan => "plan",
                crate::types::PermissionMode::BypassPermissions => "bypassPermissions",
            };
            cmd.arg("--permission-mode").arg(mode_str);

            // BypassPermissions requires the dangerous skip flag
            if matches!(mode, crate::types::PermissionMode::BypassPermissions) {
                cmd.arg("--dangerously-skip-permissions");
            }
        }

        // Continue conversation
        if self.options.continue_conversation {
            cmd.arg("--continue");
        }

        // Resume session
        if let Some(ref session_id) = self.options.resume {
            cmd.arg("--resume").arg(session_id.as_str());
        }

        // Settings file
        if let Some(ref settings) = self.options.settings {
            cmd.arg("--settings").arg(settings);
        }

        // Add directories
        for dir in &self.options.add_dirs {
            cmd.arg("--add-dir").arg(dir);
        }

        // MCP servers
        match &self.options.mcp_servers {
            crate::types::McpServers::Dict(servers) => {
                if !servers.is_empty() {
                    let mut config_map = HashMap::new();
                    for (name, config) in servers {
                        config_map.insert(name.clone(), Self::serialize_mcp_config(config));
                    }
                    let config_json = serde_json::json!({
                        "mcpServers": config_map
                    });
                    cmd.arg("--mcp-config").arg(config_json.to_string());
                }
            }
            crate::types::McpServers::Path(path) => {
                cmd.arg("--mcp-config").arg(path);
            }
            crate::types::McpServers::None => {}
        }

        // Include partial messages
        if self.options.include_partial_messages {
            cmd.arg("--include-partial-messages");
        }

        // Fork session
        if self.options.fork_session {
            cmd.arg("--fork-session");
        }

        // Custom session ID
        if let Some(ref session_id) = self.options.session_id {
            cmd.arg("--session-id").arg(session_id);
        }

        // Agents
        if let Some(ref agents) = self.options.agents {
            let agents_json = serde_json::to_string(agents).unwrap_or_default();
            cmd.arg("--agents").arg(agents_json);
        }

        // Setting sources
        if let Some(ref sources) = self.options.setting_sources {
            let sources_str: Vec<&str> = sources
                .iter()
                .map(|s| match s {
                    crate::types::SettingSource::User => "user",
                    crate::types::SettingSource::Project => "project",
                    crate::types::SettingSource::Local => "local",
                })
                .collect();
            cmd.arg("--setting-sources").arg(sources_str.join(","));
        } else {
            cmd.arg("--setting-sources").arg("");
        }

        // User identifier
        if let Some(ref user) = self.options.user {
            cmd.arg("--user").arg(user);
        }

        // ====================================================================
        // New options for TypeScript SDK parity
        // ====================================================================

        // Max budget in USD
        if let Some(max_budget) = self.options.max_budget_usd {
            cmd.arg("--max-budget-usd").arg(max_budget.to_string());
        }

        // Max thinking tokens
        if let Some(max_thinking) = self.options.max_thinking_tokens {
            cmd.arg("--max-thinking-tokens")
                .arg(max_thinking.to_string());
        }

        // Fallback model
        if let Some(ref fallback) = self.options.fallback_model {
            cmd.arg("--fallback-model").arg(fallback);
        }

        // JSON schema for structured outputs
        if let Some(ref output_format) = self.options.output_format {
            let schema_json = serde_json::to_string(&output_format.schema).unwrap_or_default();
            tracing::debug!("Adding --json-schema flag with schema: {}", schema_json);
            cmd.arg("--json-schema").arg(schema_json);
        }

        // Sandbox settings
        if let Some(ref sandbox) = self.options.sandbox
            && sandbox.enabled == Some(true)
        {
            cmd.arg("--sandbox");

            if sandbox.auto_allow_bash_if_sandboxed == Some(true) {
                cmd.arg("--sandbox-auto-allow-bash");
            }

            if let Some(ref excluded) = sandbox.excluded_commands {
                cmd.arg("--sandbox-excluded-commands")
                    .arg(excluded.join(","));
            }

            if sandbox.allow_unsandboxed_commands == Some(true) {
                cmd.arg("--sandbox-allow-unsandboxed");
            }

            if sandbox.enable_weaker_nested_sandbox == Some(true) {
                cmd.arg("--sandbox-weaker-nested");
            }

            // Network settings
            if let Some(ref network) = sandbox.network {
                if network.allow_local_binding == Some(true) {
                    cmd.arg("--sandbox-allow-local-binding");
                }
                if network.allow_all_unix_sockets == Some(true) {
                    cmd.arg("--sandbox-allow-all-unix-sockets");
                }
                if let Some(ref unix_sockets) = network.allow_unix_sockets {
                    cmd.arg("--sandbox-allow-unix-sockets")
                        .arg(unix_sockets.join(","));
                }
                if let Some(port) = network.http_proxy_port {
                    cmd.arg("--sandbox-http-proxy-port").arg(port.to_string());
                }
                if let Some(port) = network.socks_proxy_port {
                    cmd.arg("--sandbox-socks-proxy-port").arg(port.to_string());
                }
            }

            // Ignore violations
            if let Some(ref ignore) = sandbox.ignore_violations {
                if let Some(ref files) = ignore.file {
                    cmd.arg("--sandbox-ignore-file-violations")
                        .arg(files.join(","));
                }
                if let Some(ref networks) = ignore.network {
                    cmd.arg("--sandbox-ignore-network-violations")
                        .arg(networks.join(","));
                }
            }
        }

        // Plugins (local paths)
        if let Some(ref plugins) = self.options.plugins {
            for plugin in plugins {
                match plugin {
                    crate::types::SdkPluginConfig::Local { path } => {
                        cmd.arg("--plugin-dir").arg(path);
                    }
                }
            }
        }

        // Beta features
        if let Some(ref betas) = self.options.betas {
            for beta in betas {
                let beta_str = match beta {
                    crate::types::SdkBeta::Context1M => "context-1m-2025-08-07",
                };
                cmd.arg("--beta").arg(beta_str);
            }
        }

        // Strict MCP config validation
        if self.options.strict_mcp_config {
            cmd.arg("--strict-mcp-config");
        }

        // File checkpointing for rewind support
        // Enabled via environment variable, not CLI flag
        if self.options.enable_file_checkpointing {
            cmd.env("CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING", "1");
        }

        // Resume session at specific message UUID
        if let Some(ref resume_at) = self.options.resume_session_at {
            cmd.arg("--resume-at").arg(resume_at);
        }

        // Extra args - strict allowlist enforcement
        // Reject any flags not in the allowlist to prevent CLI injection
        let disallowed: Vec<&String> = self
            .options
            .extra_args
            .keys()
            .filter(|flag| !ALLOWED_EXTRA_FLAGS.contains(&flag.as_str()))
            .collect();

        if !disallowed.is_empty() {
            let flags_str = disallowed
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            tracing::warn!(
                flags = %flags_str,
                allowed = ?ALLOWED_EXTRA_FLAGS,
                "Rejected disallowed CLI flags in extra_args"
            );
            return Err(crate::error::ClaudeError::invalid_config(format!(
                "Disallowed CLI flags in extra_args: [{flags_str}]. Allowed flags: {ALLOWED_EXTRA_FLAGS:?}"
            )));
        }

        // All flags are allowed, add them
        for (flag, value) in &self.options.extra_args {
            if let Some(v) = value {
                cmd.arg(format!("--{flag}")).arg(v);
            } else {
                cmd.arg(format!("--{flag}"));
            }
        }

        // Prompt handling based on mode
        match &self.prompt {
            PromptInput::Stream => {
                // Streaming mode: use --input-format stream-json
                // --replay-user-messages enables CLI to read stdin during streaming
                cmd.arg("--input-format").arg("stream-json");
                cmd.arg("--replay-user-messages");
            }
            PromptInput::String(s) => {
                // String mode: pass the prompt as an argument after --
                cmd.arg("--").arg(s);
            }
        }

        Ok(cmd)
    }
}
