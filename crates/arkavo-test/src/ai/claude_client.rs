use crate::{Result, TestError};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone)]
pub struct ClaudeClient {
    client: Client,
    api_key: String,
    model: String,
}

#[derive(Serialize)]
struct ClaudeRequest {
    model: String,
    messages: Vec<Message>,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Serialize, Deserialize)]
pub struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ClaudeResponse {
    content: Vec<Content>,
}

#[derive(Deserialize)]
struct Content {
    text: String,
}

impl ClaudeClient {
    pub fn new(api_key: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("Failed to create HTTP client");
        
        Self {
            client,
            api_key,
            model: "claude-3-sonnet-20240229".to_string(),
        }
    }
    
    pub fn with_model(mut self, model: String) -> Self {
        self.model = model;
        self
    }
    
    pub async fn complete(&self, prompt: &str) -> Result<String> {
        let request = ClaudeRequest {
            model: self.model.clone(),
            messages: vec![Message {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
            max_tokens: 4096,
            temperature: 0.0,
        };
        
        let response = self.client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| TestError::Ai(format!("Failed to send request to Claude: {}", e)))?;
        
        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(TestError::Ai(format!(
                "Claude API error: {} - {}",
                status, error_text
            )));
        }
        
        let claude_response: ClaudeResponse = response
            .json()
            .await
            .map_err(|e| TestError::Ai(format!("Failed to parse Claude response: {}", e)))?;
        
        claude_response
            .content
            .first()
            .map(|c| c.text.clone())
            .ok_or_else(|| TestError::Ai("Empty response from Claude".to_string()))
    }
    
    pub async fn complete_with_context(
        &self,
        prompt: &str,
        context: Vec<Message>,
    ) -> Result<String> {
        let mut messages = context;
        messages.push(Message {
            role: "user".to_string(),
            content: prompt.to_string(),
        });
        
        let request = ClaudeRequest {
            model: self.model.clone(),
            messages,
            max_tokens: 4096,
            temperature: 0.0,
        };
        
        let response = self.client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| TestError::Ai(format!("Failed to send request to Claude: {}", e)))?;
        
        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(TestError::Ai(format!(
                "Claude API error: {} - {}",
                status, error_text
            )));
        }
        
        let claude_response: ClaudeResponse = response
            .json()
            .await
            .map_err(|e| TestError::Ai(format!("Failed to parse Claude response: {}", e)))?;
        
        claude_response
            .content
            .first()
            .map(|c| c.text.clone())
            .ok_or_else(|| TestError::Ai("Empty response from Claude".to_string()))
    }
}