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
use crate::policy_bridge::PolicyBridge;
use crate::sdk_bridge::SdkBridge;
use crate::{ClaudeCodeError, Result};

/// Claude Code capability that integrates the native Rust Claude Agent SDK.
///
/// Supports bidirectional sessions via `ClaudeSDKClient` with hooks and
/// permissions, falling back to `query()` for one-shot interactions.
pub struct ClaudeCodeCapability {
    config: Arc<RwLock<ClaudeCodeConfig>>,
    sdk_bridge: Arc<RwLock<Option<SdkBridge>>>,
    event_writer: Arc<EventWriter>,
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
        auth_client: Option<Arc<AuthorizationClient>>,
    ) -> Result<Self> {
        config.validate()?;

        let event_mapper = Arc::new(EventMapper::new(agent_id.clone(), event_writer.clone()));
        let policy_bridge = Arc::new(PolicyBridge::new(config.clone(), auth_client));

        // Create SDK bridge with policy enforcement and hooks
        let bridge = SdkBridge::new(&config, event_mapper, Some(policy_bridge))?;
        let bridge_lock = Arc::new(RwLock::new(Some(bridge)));

        Ok(Self {
            config: Arc::new(RwLock::new(config)),
            sdk_bridge: bridge_lock,
            event_writer,
            budget_tracker,
            session_id: Uuid::new_v4().to_string(),
            agent_id,
        })
    }

    /// Prepare the capability (authenticate and start session)
    #[allow(clippy::significant_drop_tightening)]
    pub async fn prepare(&self) -> Result<()> {
        let bridge = self.sdk_bridge.read().await;
        let bridge = bridge
            .as_ref()
            .ok_or_else(|| ClaudeCodeError::Other("SDK bridge not initialized".to_string()))?;

        let config = self.config.read().await;
        if !config.enabled {
            return Err(ClaudeCodeError::Configuration(
                "Claude Code capability is disabled".to_string(),
            ));
        }
        let workspace = config.workspace_root.clone();
        drop(config);

        info!(
            "Preparing Claude Code capability with workspace: {}",
            workspace.display()
        );

        // Authenticate (OAuth or API key)
        bridge.initialize().await?;

        // Start bidirectional session if configured
        bridge.start_session().await?;

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
                        "bidirectional".to_string(),
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
    #[allow(clippy::significant_drop_tightening)]
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

        let run_id = format!("run-{}", Uuid::new_v4());
        debug!("Starting Claude Code run: {run_id}");

        let full_prompt = if let Some(ctx) = context {
            format!(
                "{prompt}\n\nContext:\n{}",
                serde_json::to_string_pretty(&ctx).unwrap_or_default()
            )
        } else {
            prompt
        };

        let bridge = self.sdk_bridge.read().await;
        let bridge = bridge
            .as_ref()
            .ok_or_else(|| ClaudeCodeError::Other("SDK bridge not initialized".to_string()))?;

        bridge.run_query(full_prompt, run_id.clone()).await?;
        Ok(run_id)
    }

    /// Interrupt the current run
    #[allow(clippy::significant_drop_tightening)]
    pub async fn stop_run(&self, _run_id: String) -> Result<()> {
        let bridge = self.sdk_bridge.read().await;
        if let Some(bridge) = bridge.as_ref() {
            bridge.interrupt().await?;
        }
        Ok(())
    }

    /// Get session info (model, tools, MCP servers, permission mode)
    pub async fn session_info(&self) -> Result<Value> {
        let bridge = self.sdk_bridge.read().await;
        match bridge.as_ref() {
            Some(b) => b.session_info().await,
            None => Ok(serde_json::json!({"status": "not initialized"})),
        }
    }

    /// Get budget status for AG-UI `ComputeBudgetUpdate` event shape
    pub fn compute_budget_status(&self) -> Value {
        let bridge_guard = self.sdk_bridge.try_read().ok();
        let (cost, input_tokens, output_tokens) = bridge_guard
            .as_ref()
            .and_then(|guard| guard.as_ref())
            .map(|b| {
                (
                    b.accumulated_cost_usd(),
                    b.accumulated_input_tokens(),
                    b.accumulated_output_tokens(),
                )
            })
            .unwrap_or((0.0, 0, 0));

        let max_budget = self
            .config
            .try_read()
            .ok()
            .as_ref()
            .and_then(|c| c.max_budget_usd);

        serde_json::json!({
            "total_cost_usd": cost,
            "max_budget_usd": max_budget,
            "remaining_cost_usd": max_budget.map(|max| (max - cost).max(0.0)),
            "used_input_tokens": input_tokens,
            "used_output_tokens": output_tokens,
            "used_tokens": input_tokens + output_tokens,
        })
    }

    /// Shutdown the capability
    pub async fn shutdown(&self) -> Result<()> {
        let bridge = self.sdk_bridge.write().await.take();
        if let Some(bridge) = bridge {
            bridge.close_session().await?;
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
            ToolSchema {
                aliases: None,
                name: "claude_code_session_info".to_string(),
                description: "Get session info: model, tools, MCP servers, permission mode"
                    .to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                }),
            },
            ToolSchema {
                aliases: None,
                name: "claude_code_interrupt".to_string(),
                description: "Interrupt the current Claude Code run".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "run_id": {
                            "type": "string",
                            "description": "The run ID to interrupt"
                        }
                    },
                    "required": ["run_id"]
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
            "claude_code_session_info" => {
                let info = self
                    .session_info()
                    .await
                    .map_err(|e| Box::<dyn std::error::Error + Send + Sync>::from(e.to_string()))?;
                Ok(serde_json::json!({
                    "success": true,
                    "session_info": info,
                    "budget_status": self.compute_budget_status(),
                }))
            }
            "claude_code_interrupt" => {
                let run_id = params["run_id"].as_str().unwrap_or("unknown").to_string();
                self.stop_run(run_id)
                    .await
                    .map_err(|e| Box::<dyn std::error::Error + Send + Sync>::from(e.to_string()))?;
                Ok(serde_json::json!({
                    "success": true,
                    "message": "Run interrupted"
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
                        "enum": ["claude_code_run", "claude_code_plan", "claude_code_session_info", "claude_code_interrupt"],
                        "description": "The specific Claude Code tool to use"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "The task prompt"
                    },
                    "context": {
                        "type": "object",
                        "description": "Optional context"
                    },
                    "run_id": {
                        "type": "string",
                        "description": "Run ID for interrupt"
                    }
                },
                "required": ["tool"]
            }),
        });
        &SCHEMA
    }
}
