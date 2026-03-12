//! MCP Tools and internal tool definitions for Claude Code
//!
//! This module provides:
//! - Internal tools (file, exec, web) used by the Claude Code capability
//! - MCP tool wrappers for exposing capabilities to the tool registry
//!
//! MCP Tools (entitlement-gated):
//! - `claude_code_run`: Execute a Claude Code task with full SDK capabilities
//! - `claude_code_plan`: Generate a plan for a coding task without execution

pub mod exec_tool;
pub mod file_tools;
pub mod web_tool;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

use arkavo_mcp::ToolSchema;
use arkavo_mcp_tools::server::Tool;

use crate::ClaudeCodeCapability;

/// Trait for internal Claude Code tools
#[async_trait]
pub trait ClaudeCodeTool: Send + Sync {
    /// Get the tool name
    fn name(&self) -> &'static str;

    /// Get the tool description
    fn description(&self) -> &'static str;

    /// Execute the tool with given parameters
    async fn execute(&self, params: Value) -> crate::Result<Value>;

    /// Check if the tool is enabled
    fn is_enabled(&self) -> bool;
}

/// Tool for executing Claude Code tasks with full SDK capabilities
pub struct ClaudeCodeRunTool {
    capability: Arc<ClaudeCodeCapability>,
    schema: ToolSchema,
}

impl ClaudeCodeRunTool {
    pub fn new(capability: Arc<ClaudeCodeCapability>) -> Self {
        Self {
            capability,
            schema: ToolSchema {
                name: "claude_code_run".to_string(),
                aliases: Some(vec!["cc_run".to_string(), "claude_run".to_string()]),
                description: "Execute a Claude Code task with full SDK capabilities. \
                              Supports file operations, code generation, and analysis. \
                              Requires entitlement: https://arkavo.net/attr/mcp-tool/value/claude_code_run"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "prompt": {
                            "type": "string",
                            "description": "The task prompt for Claude Code to execute"
                        },
                        "context": {
                            "type": "object",
                            "description": "Optional context for the task (e.g., file paths, constraints)"
                        }
                    },
                    "required": ["prompt"]
                }),
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct RunArgs {
    prompt: String,
    #[serde(default)]
    context: Option<Value>,
}

#[derive(Debug, Serialize)]
struct RunResponse {
    success: bool,
    run_id: String,
    message: String,
}

#[async_trait]
impl Tool for ClaudeCodeRunTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> arkavo_mcp_tools::Result<Value> {
        let args: RunArgs =
            serde_json::from_value(args).map_err(arkavo_mcp_tools::ToolError::Json)?;

        let run_id = self
            .capability
            .start_run(args.prompt, args.context)
            .await
            .map_err(|e| arkavo_mcp_tools::ToolError::Execution(e.to_string()))?;

        let response = RunResponse {
            success: true,
            run_id,
            message: "Run completed successfully".to_string(),
        };

        serde_json::to_value(response).map_err(arkavo_mcp_tools::ToolError::Json)
    }
}

/// Tool for generating plans without execution
pub struct ClaudeCodePlanTool {
    capability: Arc<ClaudeCodeCapability>,
    schema: ToolSchema,
}

impl ClaudeCodePlanTool {
    pub fn new(capability: Arc<ClaudeCodeCapability>) -> Self {
        Self {
            capability,
            schema: ToolSchema {
                name: "claude_code_plan".to_string(),
                aliases: Some(vec!["cc_plan".to_string(), "claude_plan".to_string()]),
                description: "Generate a detailed plan for a coding task without execution. \
                              Returns step-by-step analysis and recommendations. \
                              Requires entitlement: https://arkavo.net/attr/mcp-tool/value/claude_code_plan"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "prompt": {
                            "type": "string",
                            "description": "The task to plan (will not execute any code)"
                        }
                    },
                    "required": ["prompt"]
                }),
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct PlanArgs {
    prompt: String,
}

#[derive(Debug, Serialize)]
struct PlanResponse {
    success: bool,
    run_id: String,
    message: String,
}

#[async_trait]
impl Tool for ClaudeCodePlanTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> arkavo_mcp_tools::Result<Value> {
        let args: PlanArgs =
            serde_json::from_value(args).map_err(arkavo_mcp_tools::ToolError::Json)?;

        // Add planning instruction to prevent execution
        let plan_prompt = format!(
            "Please create a detailed plan for the following task. \
             Do not execute any code, just outline the steps:\n\n{}",
            args.prompt
        );

        let run_id = self
            .capability
            .start_run(plan_prompt, None)
            .await
            .map_err(|e| arkavo_mcp_tools::ToolError::Execution(e.to_string()))?;

        let response = PlanResponse {
            success: true,
            run_id,
            message: "Planning completed successfully".to_string(),
        };

        serde_json::to_value(response).map_err(arkavo_mcp_tools::ToolError::Json)
    }
}

/// Tool for querying session info and budget status
pub struct ClaudeCodeSessionInfoTool {
    capability: Arc<ClaudeCodeCapability>,
    schema: ToolSchema,
}

impl ClaudeCodeSessionInfoTool {
    pub fn new(capability: Arc<ClaudeCodeCapability>) -> Self {
        Self {
            capability,
            schema: ToolSchema {
                name: "claude_code_session_info".to_string(),
                aliases: Some(vec!["cc_info".to_string()]),
                description: "Get Claude Code session info: model, tools, MCP servers, budget"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for ClaudeCodeSessionInfoTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, _args: Value) -> arkavo_mcp_tools::Result<Value> {
        let info = self
            .capability
            .session_info()
            .await
            .map_err(|e| arkavo_mcp_tools::ToolError::Execution(e.to_string()))?;

        Ok(json!({
            "success": true,
            "session_info": info,
            "budget_status": self.capability.compute_budget_status(),
        }))
    }
}

/// Tool for interrupting a running Claude Code task
pub struct ClaudeCodeInterruptTool {
    capability: Arc<ClaudeCodeCapability>,
    schema: ToolSchema,
}

impl ClaudeCodeInterruptTool {
    pub fn new(capability: Arc<ClaudeCodeCapability>) -> Self {
        Self {
            capability,
            schema: ToolSchema {
                name: "claude_code_interrupt".to_string(),
                aliases: Some(vec!["cc_interrupt".to_string()]),
                description: "Interrupt a running Claude Code task".to_string(),
                parameters: json!({
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
        }
    }
}

#[derive(Debug, Deserialize)]
struct InterruptArgs {
    run_id: String,
}

#[async_trait]
impl Tool for ClaudeCodeInterruptTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> arkavo_mcp_tools::Result<Value> {
        let args: InterruptArgs =
            serde_json::from_value(args).map_err(arkavo_mcp_tools::ToolError::Json)?;

        self.capability
            .stop_run(args.run_id.clone())
            .await
            .map_err(|e| arkavo_mcp_tools::ToolError::Execution(e.to_string()))?;

        Ok(json!({
            "success": true,
            "message": format!("Run {} interrupted", args.run_id)
        }))
    }
}

/// Register Claude Code tools with a ToolRegistry
///
/// Tools registered:
/// - `claude_code_run`: Execute tasks with full Claude Code capabilities
/// - `claude_code_plan`: Generate plans without execution
/// - `claude_code_session_info`: Get session info and budget status
/// - `claude_code_interrupt`: Interrupt a running task
pub fn register_tools(
    registry: &mut arkavo_mcp_tools::ToolRegistry,
    capability: Arc<ClaudeCodeCapability>,
) {
    registry.register(
        "claude_code_run",
        Box::new(ClaudeCodeRunTool::new(Arc::clone(&capability))),
    );
    registry.register(
        "claude_code_plan",
        Box::new(ClaudeCodePlanTool::new(Arc::clone(&capability))),
    );
    registry.register(
        "claude_code_session_info",
        Box::new(ClaudeCodeSessionInfoTool::new(Arc::clone(&capability))),
    );
    registry.register(
        "claude_code_interrupt",
        Box::new(ClaudeCodeInterruptTool::new(capability)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_tool_schema() {
        let schema = ToolSchema {
            name: "claude_code_run".to_string(),
            aliases: Some(vec!["cc_run".to_string(), "claude_run".to_string()]),
            description: "Execute a Claude Code task".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "prompt": {"type": "string"}
                },
                "required": ["prompt"]
            }),
        };

        assert_eq!(schema.name, "claude_code_run");
        assert!(schema.description.contains("Claude Code"));
    }

    #[test]
    fn test_plan_tool_schema() {
        let schema = ToolSchema {
            name: "claude_code_plan".to_string(),
            aliases: Some(vec!["cc_plan".to_string(), "claude_plan".to_string()]),
            description: "Generate a plan".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "prompt": {"type": "string"}
                },
                "required": ["prompt"]
            }),
        };

        assert_eq!(schema.name, "claude_code_plan");
    }

    #[test]
    fn test_run_args_deserialization() {
        let json = json!({
            "prompt": "Write a function",
            "context": {"file": "src/main.rs"}
        });

        let args: RunArgs = serde_json::from_value(json).unwrap();
        assert_eq!(args.prompt, "Write a function");
        assert!(args.context.is_some());
    }

    #[test]
    fn test_run_args_without_context() {
        let json = json!({
            "prompt": "Write a function"
        });

        let args: RunArgs = serde_json::from_value(json).unwrap();
        assert_eq!(args.prompt, "Write a function");
        assert!(args.context.is_none());
    }

    #[test]
    fn test_plan_args_deserialization() {
        let json = json!({
            "prompt": "Plan a refactoring"
        });

        let args: PlanArgs = serde_json::from_value(json).unwrap();
        assert_eq!(args.prompt, "Plan a refactoring");
    }
}
