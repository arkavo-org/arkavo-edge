use async_trait::async_trait;
use serde_json::Value;
use std::sync::{Arc, LazyLock};
use tokio::sync::RwLock;
use tracing::{debug, info};
use uuid::Uuid;

use arkavo_authorization::AuthorizationClient;
use arkavo_budget::BudgetTracker;
use arkavo_events::{Event, EventPayload, EventWriter};
use arkavo_mcp::{Tool, ToolSchema};

use crate::config::ClaudeCodeConfig;
use crate::event_mapper::EventMapper;
use crate::sdk_bridge::SdkBridge;
use crate::{ClaudeCodeError, Result};

/// Claude Code capability that integrates the native Rust Claude Agent SDK
pub struct ClaudeCodeCapability {
    config: Arc<RwLock<ClaudeCodeConfig>>,
    sdk_bridge: Arc<RwLock<Option<SdkBridge>>>,
    event_writer: Arc<EventWriter>,
    event_mapper: Arc<EventMapper>,
    budget_tracker: Option<Arc<BudgetTracker>>,
    session_id: String,
    agent_id: String,
}

impl ClaudeCodeCapability {
    /// Create a new Claude Code capability
    pub fn new(
        config: ClaudeCodeConfig,
        agent_id: String,
        event_writer: Arc<EventWriter>,
        budget_tracker: Option<Arc<BudgetTracker>>,
        _auth_client: Option<Arc<AuthorizationClient>>,
    ) -> Result<Self> {
        config.validate()?;

        let event_mapper = Arc::new(EventMapper::new(agent_id.clone(), event_writer.clone()));

        Ok(Self {
            config: Arc::new(RwLock::new(config)),
            sdk_bridge: Arc::new(RwLock::new(None)),
            event_writer,
            event_mapper,
            budget_tracker,
            session_id: Uuid::new_v4().to_string(),
            agent_id,
        })
    }

    /// Prepare the capability (initialize SDK and authenticate)
    pub async fn prepare(&self) -> Result<()> {
        // Create SDK bridge with config read lock scoped tightly
        let (bridge, workspace_root) = {
            let config = self.config.read().await;

            if !config.enabled {
                return Err(ClaudeCodeError::Configuration(
                    "Claude Code capability is disabled".to_string(),
                ));
            }

            let workspace = config.workspace_root.clone();
            let bridge = SdkBridge::new(&config, self.event_mapper.clone())?;
            drop(config);
            (bridge, workspace)
        };

        info!("Preparing Claude Code capability with workspace: {}", workspace_root.display());

        // Initialize authentication (OAuth or API key)
        bridge.initialize().await?;

        // Store the bridge (write lock scoped tightly)
        *self.sdk_bridge.write().await = Some(bridge);

        // Emit session started event
        self.event_writer
            .write(Event::new(
                self.session_id.clone(),
                0,
                self.agent_id.clone(),
                EventPayload::SessionStarted {
                    capabilities: Some(vec![
                        "claude_code".to_string(),
                        "file_operations".to_string(),
                        "code_generation".to_string(),
                    ]),
                    metadata: None,
                },
            ))
            .await
            .map_err(|e| ClaudeCodeError::Other(e.to_string()))?;

        info!("Claude Code capability prepared successfully");
        Ok(())
    }

    /// Start a Claude Code run with streaming
    pub async fn start_run(&self, prompt: String, context: Option<Value>) -> Result<String> {
        // Check budget if tracker is available
        if let Some(tracker) = &self.budget_tracker {
            let estimated_cost = arkavo_budget::TokenCost::from_dollars(0.01);

            let can_afford = tracker
                .can_afford(&self.agent_id, estimated_cost)
                .await
                .map_err(|e| ClaudeCodeError::BudgetExceeded(e.to_string()))?;

            if !can_afford {
                return Err(ClaudeCodeError::BudgetExceeded(
                    "Insufficient budget for this operation".to_string(),
                ));
            }
        }

        // Generate a run ID
        let run_id = format!("run-{}", Uuid::new_v4());

        debug!("Starting Claude Code run: {run_id}");

        // Build the full prompt with context if provided
        let full_prompt = if let Some(ctx) = context {
            format!(
                "{prompt}\n\nContext:\n{}",
                serde_json::to_string_pretty(&ctx).unwrap_or_default()
            )
        } else {
            prompt
        };

        // Start the query run (read lock scoped tightly)
        self.sdk_bridge
            .read()
            .await
            .as_ref()
            .ok_or_else(|| ClaudeCodeError::Other("SDK bridge not initialized".to_string()))?
            .run_query(full_prompt, run_id.clone())
            .await?;

        Ok(run_id)
    }

    /// Stop an active run (not directly supported by query API, sessions handle this)
    pub fn stop_run(&self, _run_id: String) -> Result<()> {
        // The query API handles completion automatically
        // For interactive sessions, we would close the session
        Ok(())
    }

    /// Shutdown the capability
    pub async fn shutdown(&self) -> Result<()> {
        // Take bridge with write lock scoped tightly
        let bridge = self.sdk_bridge.write().await.take();
        if let Some(bridge) = bridge
            && let Err(e) = bridge.close_session()
        {
            tracing::warn!("Error closing Claude session: {e}");
        }

        info!("Claude Code capability shut down");
        Ok(())
    }

    /// List available tools
    pub fn list_tools(&self) -> Vec<ToolSchema> {
        vec![
            ToolSchema {
                aliases: None,
                name: "claude_code_run".to_string(),
                description: "Execute a Claude Code task with full SDK capabilities".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "prompt": {
                            "type": "string",
                            "description": "The task prompt for Claude Code"
                        },
                        "context": {
                            "type": "object",
                            "description": "Optional context for the task"
                        }
                    },
                    "required": ["prompt"]
                }),
            },
            ToolSchema {
                aliases: None,
                name: "claude_code_plan".to_string(),
                description: "Generate a plan for a coding task without execution".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "prompt": {
                            "type": "string",
                            "description": "The task to plan"
                        }
                    },
                    "required": ["prompt"]
                }),
            },
        ]
    }
}

#[async_trait]
impl Tool for ClaudeCodeCapability {
    async fn execute(
        &self,
        params: Value,
    ) -> std::result::Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let tool_name = params["tool"]
            .as_str()
            .ok_or_else(|| Box::<dyn std::error::Error + Send + Sync>::from("Missing tool name"))?;

        match tool_name {
            "claude_code_run" => {
                let prompt = params["prompt"]
                    .as_str()
                    .ok_or_else(|| {
                        Box::<dyn std::error::Error + Send + Sync>::from("Missing prompt")
                    })?
                    .to_string();

                let context = params.get("context").cloned();

                let run_id = self
                    .start_run(prompt, context)
                    .await
                    .map_err(|e| Box::<dyn std::error::Error + Send + Sync>::from(e.to_string()))?;

                Ok(serde_json::json!({
                    "success": true,
                    "message": "Run completed",
                    "run_id": run_id
                }))
            }
            "claude_code_plan" => {
                let prompt = params["prompt"]
                    .as_str()
                    .ok_or_else(|| {
                        Box::<dyn std::error::Error + Send + Sync>::from("Missing prompt")
                    })?
                    .to_string();

                // For plan mode, we add a planning instruction
                let plan_prompt = format!(
                    "Please create a detailed plan for the following task. \
                     Do not execute any code, just outline the steps:\n\n{prompt}"
                );

                let run_id = self
                    .start_run(plan_prompt, None)
                    .await
                    .map_err(|e| Box::<dyn std::error::Error + Send + Sync>::from(e.to_string()))?;

                Ok(serde_json::json!({
                    "success": true,
                    "message": "Planning completed",
                    "run_id": run_id
                }))
            }
            _ => Err(Box::<dyn std::error::Error + Send + Sync>::from(format!(
                "Unknown tool: {tool_name}"
            ))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        static SCHEMA: LazyLock<ToolSchema> = LazyLock::new(|| ToolSchema {
            name: "claude_code".to_string(),
            aliases: None,
            description: "Claude Code SDK integration for advanced coding tasks".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "tool": {
                        "type": "string",
                        "enum": ["claude_code_run", "claude_code_plan"],
                        "description": "The specific Claude Code tool to use"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "The task prompt"
                    },
                    "context": {
                        "type": "object",
                        "description": "Optional context"
                    }
                },
                "required": ["tool", "prompt"]
            }),
        });
        &SCHEMA
    }
}
