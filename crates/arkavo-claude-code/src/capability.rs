use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};
use uuid::Uuid;

use arkavo_authorization::AuthorizationClient;
use arkavo_budget::BudgetTracker;
use arkavo_events::{Event, EventPayload, EventWriter};
use arkavo_mcp_core::{Tool, ToolSchema};

use crate::config::ClaudeCodeConfig;
use crate::event_mapper::EventMapper;
use crate::node_bridge::NodeBridge;
use crate::policy_bridge::PolicyBridge;
use crate::{ClaudeCodeError, Result};

/// Claude Code capability that integrates the Claude Code SDK
pub struct ClaudeCodeCapability {
    config: Arc<RwLock<ClaudeCodeConfig>>,
    node_bridge: Arc<RwLock<Option<NodeBridge>>>,
    policy_bridge: Arc<PolicyBridge>,
    event_writer: Arc<EventWriter>,
    event_mapper: Arc<EventMapper>,
    budget_tracker: Option<Arc<BudgetTracker>>,
    session_id: String,
    agent_id: String,
}

impl ClaudeCodeCapability {
    /// Create a new Claude Code capability
    pub async fn new(
        config: ClaudeCodeConfig,
        agent_id: String,
        event_writer: Arc<EventWriter>,
        budget_tracker: Option<Arc<BudgetTracker>>,
        auth_client: Option<Arc<AuthorizationClient>>,
    ) -> Result<Self> {
        config.validate()?;

        let policy_bridge = Arc::new(PolicyBridge::new(config.clone(), auth_client.clone()));

        let event_mapper = Arc::new(EventMapper::new(agent_id.clone(), event_writer.clone()));

        Ok(Self {
            config: Arc::new(RwLock::new(config)),
            node_bridge: Arc::new(RwLock::new(None)),
            policy_bridge,
            event_writer,
            event_mapper,
            budget_tracker,
            session_id: Uuid::new_v4().to_string(),
            agent_id,
        })
    }

    /// Prepare the capability (initialize Node.js subprocess)
    pub async fn prepare(&self) -> Result<()> {
        let config = self.config.read().await;

        if !config.enabled {
            return Err(ClaudeCodeError::Configuration(
                "Claude Code capability is disabled".to_string(),
            ));
        }

        // Check for API key or auth token
        if config.anthropic_auth_token.is_none() && std::env::var("ANTHROPIC_API_KEY").is_err() {
            return Err(ClaudeCodeError::Configuration(
                "Authentication not configured.\n\
                 Please set either:\n\
                 - ANTHROPIC_API_KEY environment variable for Claude\n\
                 - anthropic_auth_token in config for DeepSeek or other providers"
                    .to_string(),
            ));
        }

        info!(
            "Preparing Claude Code capability with model: {}",
            config.anthropic_model
        );

        // Create and start Node.js bridge
        let bridge = NodeBridge::new(&config, self.event_mapper.clone()).await?;

        // Initialize the bridge - this will check for Node.js and Claude Code SDK
        match bridge.initialize().await {
            Ok(_) => {}
            Err(e) => {
                return Err(ClaudeCodeError::Configuration(format!(
                    "Claude Code capability initialization failed:\n{}\n\n\
                     To enable Claude Code capability:\n\
                     1. Install Node.js >= 18.0.0 from https://nodejs.org/\n\
                     2. Install Claude Code SDK: npm install -g @anthropic-ai/claude-code\n\
                     3. Set ANTHROPIC_API_KEY environment variable\n\
                     4. Restart Arkavo",
                    e
                )));
            }
        }

        // Store the bridge
        let mut bridge_guard = self.node_bridge.write().await;
        *bridge_guard = Some(bridge);

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
            // Estimate cost for this operation (rough estimate)
            let estimated_cost = arkavo_budget::TokenCost::from_dollars(0.01); // Estimate $0.01 per run

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

        let bridge_guard = self.node_bridge.read().await;
        let bridge = bridge_guard
            .as_ref()
            .ok_or_else(|| ClaudeCodeError::Other("Node bridge not initialized".to_string()))?;

        // Set up tool interceptor for policy enforcement
        let policy_bridge = self.policy_bridge.clone();
        bridge.set_tool_interceptor(move |tool_request| {
            let policy_bridge = policy_bridge.clone();
            Box::pin(async move { policy_bridge.check_tool_permission(tool_request).await })
        });

        // Generate a run ID
        let run_id = format!("run-{}", Uuid::new_v4());

        debug!("Starting Claude Code run: {}", run_id);

        // Start the streaming run
        bridge.start_run(prompt, run_id.clone(), context).await?;

        Ok(run_id)
    }

    /// Stop an active run
    pub async fn stop_run(&self, run_id: String) -> Result<()> {
        let bridge_guard = self.node_bridge.read().await;
        if let Some(bridge) = bridge_guard.as_ref() {
            bridge.stop_run(run_id).await?;
        }
        Ok(())
    }

    /// Shutdown the capability
    pub async fn shutdown(&self) -> Result<()> {
        let mut bridge_guard = self.node_bridge.write().await;
        if let Some(bridge) = bridge_guard.take() {
            bridge.shutdown().await?;
        }

        info!("Claude Code capability shut down");
        Ok(())
    }

    /// List available tools
    pub async fn list_tools(&self) -> Vec<ToolSchema> {
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
    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let tool_name = params["tool"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing tool name"))?;

        match tool_name {
            "claude_code_run" => {
                let prompt = params["prompt"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing prompt"))?
                    .to_string();

                let context = params.get("context").cloned();

                // Start a streaming run
                let run_id = self.start_run(prompt, context).await?;

                Ok(serde_json::json!({
                    "success": true,
                    "message": "Run started",
                    "run_id": run_id
                }))
            }
            "claude_code_plan" => {
                let prompt = params["prompt"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing prompt"))?
                    .to_string();

                let mut context = params
                    .get("context")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));

                // Set plan-only mode in context
                context["options"] = serde_json::json!({
                    "planOnly": true
                });

                // Start a streaming run in plan-only mode
                let run_id = self.start_run(prompt, Some(context)).await?;

                Ok(serde_json::json!({
                    "success": true,
                    "message": "Planning started",
                    "run_id": run_id
                }))
            }
            _ => Err(anyhow::anyhow!("Unknown tool: {}", tool_name)),
        }
    }

    fn schema(&self) -> &ToolSchema {
        // Return a composite schema for the capability
        static SCHEMA: once_cell::sync::Lazy<ToolSchema> =
            once_cell::sync::Lazy::new(|| ToolSchema {
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

// Add once_cell to dependencies
use once_cell;
