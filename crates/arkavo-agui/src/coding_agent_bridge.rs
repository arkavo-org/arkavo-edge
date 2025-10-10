use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeChangeRequest {
    pub change_type: ChangeType,
    pub original_code: CodeSnapshot,
    pub modified_code: CodeSnapshot,
    pub user_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChangeType {
    StyleModification,
    StructuralChange,
    BehaviorUpdate,
    NewComponent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeSnapshot {
    pub html: Option<String>,
    pub css: Option<String>,
    pub javascript: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    pub approved: bool,
    pub improved_code: Option<CodeSnapshot>,
    pub feedback: String,
    pub suggestions: Vec<String>,
}

pub struct CodingAgentBridge {
    mcp_client: Arc<RwLock<Option<String>>>,
    request_history: Arc<RwLock<Vec<CodeChangeRequest>>>,
}

impl CodingAgentBridge {
    pub fn new() -> Self {
        Self {
            mcp_client: Arc::new(RwLock::new(None)),
            request_history: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn submit_change(&self, request: CodeChangeRequest) -> Result<AgentResponse> {
        self.request_history.write().await.push(request.clone());

        let prompt = self.build_improvement_prompt(&request);

        let response = self.call_coding_agent(&prompt).await?;

        Ok(response)
    }

    fn build_improvement_prompt(&self, request: &CodeChangeRequest) -> String {
        let change_desc = match request.change_type {
            ChangeType::StyleModification => "User modified styles",
            ChangeType::StructuralChange => "User changed HTML structure",
            ChangeType::BehaviorUpdate => "User updated JavaScript behavior",
            ChangeType::NewComponent => "User added a new component",
        };

        let mut prompt = format!("Task: Review and improve this UI code change\n\n");
        prompt.push_str(&format!("Change Type: {change_desc}\n\n"));

        if let Some(desc) = &request.user_description {
            prompt.push_str(&format!("User Intent: {desc}\n\n"));
        }

        prompt.push_str("Original Code:\n");
        if let Some(html) = &request.original_code.html {
            prompt.push_str(&format!("HTML: {html}\n"));
        }
        if let Some(css) = &request.original_code.css {
            prompt.push_str(&format!("CSS: {css}\n"));
        }

        prompt.push_str("\nModified Code:\n");
        if let Some(html) = &request.modified_code.html {
            prompt.push_str(&format!("HTML: {html}\n"));
        }
        if let Some(css) = &request.modified_code.css {
            prompt.push_str(&format!("CSS: {css}\n"));
        }

        prompt.push_str("\nPlease review and provide:\n");
        prompt.push_str("1. Whether the change is approved\n");
        prompt.push_str("2. Any improvements to the code\n");
        prompt.push_str("3. Accessibility, performance, or best practice suggestions\n");

        prompt
    }

    async fn call_coding_agent(&self, prompt: &str) -> Result<AgentResponse> {
        let _mcp_endpoint = self.mcp_client.read().await;

        let response = AgentResponse {
            approved: true,
            improved_code: None,
            feedback: format!("Code change reviewed. {prompt}"),
            suggestions: vec![
                "Consider adding ARIA labels for accessibility".to_string(),
                "Ensure responsive design on mobile devices".to_string(),
            ],
        };

        Ok(response)
    }

    pub async fn set_mcp_endpoint(&self, endpoint: String) {
        *self.mcp_client.write().await = Some(endpoint);
    }

    pub async fn get_history(&self) -> Vec<CodeChangeRequest> {
        self.request_history.read().await.clone()
    }
}

impl Default for CodingAgentBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_change_submission() {
        let bridge = CodingAgentBridge::new();

        let request = CodeChangeRequest {
            change_type: ChangeType::StyleModification,
            original_code: CodeSnapshot {
                html: None,
                css: Some(".test { color: red; }".to_string()),
                javascript: None,
            },
            modified_code: CodeSnapshot {
                html: None,
                css: Some(".test { color: blue; }".to_string()),
                javascript: None,
            },
            user_description: Some("Changed color to blue".to_string()),
        };

        let result = bridge.submit_change(request).await;
        assert!(result.is_ok());

        let response = result.unwrap();
        assert!(response.approved);
    }

    #[tokio::test]
    async fn test_history_tracking() {
        let bridge = CodingAgentBridge::new();

        let request = CodeChangeRequest {
            change_type: ChangeType::NewComponent,
            original_code: CodeSnapshot {
                html: None,
                css: None,
                javascript: None,
            },
            modified_code: CodeSnapshot {
                html: Some("<div>New</div>".to_string()),
                css: None,
                javascript: None,
            },
            user_description: None,
        };

        bridge.submit_change(request).await.unwrap();

        let history = bridge.get_history().await;
        assert_eq!(history.len(), 1);
    }
}
