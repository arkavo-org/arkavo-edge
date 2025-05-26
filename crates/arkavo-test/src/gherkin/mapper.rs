use crate::{Result, TestError};
use crate::ai::claude_client::ClaudeClient;
use crate::gherkin::parser::Step;
use crate::mcp::tools::{Action, ToolDefinition};
use std::collections::HashMap;

pub struct AIStepMapper {
    claude_client: ClaudeClient,
    tool_registry: HashMap<String, ToolDefinition>,
}

impl AIStepMapper {
    pub fn new(claude_client: ClaudeClient) -> Self {
        Self {
            claude_client,
            tool_registry: HashMap::new(),
        }
    }
    
    pub fn register_tool(&mut self, tool: ToolDefinition) {
        self.tool_registry.insert(tool.name.clone(), tool);
    }
    
    pub async fn map_step_to_actions(&self, step: &Step) -> Result<Vec<Action>> {
        let tools_list: Vec<&ToolDefinition> = self.tool_registry.values().collect();
        
        let prompt = self.build_mapping_prompt(step, &tools_list);
        
        let response = self.claude_client
            .complete(&prompt)
            .await
            .map_err(|e| TestError::Ai(format!("Failed to get AI response: {}", e)))?;
        
        self.parse_actions_response(&response)
    }
    
    fn build_mapping_prompt(&self, step: &Step, tools: &[&ToolDefinition]) -> String {
        let tools_json = serde_json::to_string_pretty(tools).unwrap_or_default();
        
        format!(
            r#"Map this BDD step to tool calls.

Step: {} {}

Available tools:
{}

Return a JSON array of actions. Each action should have:
- tool_name: The name of the tool to call
- parameters: The parameters to pass to the tool
- expected_outcome: Optional description of what should happen

Example response:
[
  {{
    "tool_name": "query_state",
    "parameters": {{
      "entity": "user_account",
      "filter": {{"balance": {{"$gt": 0}}}}
    }},
    "expected_outcome": "User account should have positive balance"
  }}
]

Return ONLY the JSON array, no other text."#,
            step.keyword,
            step.text,
            tools_json
        )
    }
    
    fn parse_actions_response(&self, response: &str) -> Result<Vec<Action>> {
        let cleaned_response = response.trim();
        
        serde_json::from_str::<Vec<Action>>(cleaned_response)
            .map_err(|e| TestError::Ai(format!("Failed to parse AI response as actions: {}", e)))
    }
    
    pub async fn natural_language_to_actions(&self, text: &str) -> Result<Vec<Action>> {
        let tools_list: Vec<&ToolDefinition> = self.tool_registry.values().collect();
        
        let prompt = format!(
            r#"Convert this natural language test description to tool calls.

Description: {}

Available tools:
{}

Return a JSON array of actions with tool_name, parameters, and optional expected_outcome.
Return ONLY the JSON array."#,
            text,
            serde_json::to_string_pretty(&tools_list).unwrap_or_default()
        );
        
        let response = self.claude_client
            .complete(&prompt)
            .await
            .map_err(|e| TestError::Ai(format!("Failed to get AI response: {}", e)))?;
        
        self.parse_actions_response(&response)
    }
}