//! Multi-stage reasoning with thinking, critique, and deliberation.
//!
//! This module implements a deliberation system that allows LLMs to:
//! 1. Think through a problem (internal reasoning)
//! 2. Critique their own response
//! 3. Refine based on judge feedback

use crate::judge::{JudgmentResult, ResponseJudge};
use arkavo_llm::{tool_executor::ToolExecutionResult, Message, Provider, ProviderResponse, Role};
use arkavo_mcp_tools::ToolInfo;
use std::sync::Arc;

/// Configuration for the deliberation process.
#[derive(Debug, Clone)]
pub struct DeliberationConfig {
    /// Enable internal thinking/reasoning before responding
    pub enable_thinking: bool,
    /// Enable self-critique pass
    pub enable_critique: bool,
    /// Enable external judge validation
    pub enable_judge: bool,
    /// Maximum number of deliberation rounds to prevent infinite loops
    pub max_rounds: u8,
}

impl Default for DeliberationConfig {
    fn default() -> Self {
        Self {
            enable_thinking: true,
            enable_critique: true,
            enable_judge: true,
            max_rounds: 3,
        }
    }
}

/// Result of the deliberation process.
#[derive(Debug, Clone)]
pub struct DeliberationResult {
    /// Internal reasoning/thinking (from models that support it)
    pub thinking: Option<String>,
    /// Self-critique of the response
    pub critique: Option<String>,
    /// The final consolidated response
    pub final_response: String,
    /// Number of iterations performed
    pub iterations: u8,
    /// Confidence score (0.0-1.0) based on critique and judge results
    pub confidence: f32,
}

/// Multi-stage reasoning orchestrator.
pub struct Deliberator {
    /// Provider for thinking/reasoning (should support reasoning_content)
    thinking_provider: Arc<dyn Provider>,
    /// Optional judge for external validation
    judge: Option<ResponseJudge>,
    /// Configuration
    config: DeliberationConfig,
}

impl Deliberator {
    /// Create a new deliberator with the given provider.
    pub fn new(thinking_provider: Arc<dyn Provider>, config: DeliberationConfig) -> Self {
        Self {
            thinking_provider,
            judge: None,
            config,
        }
    }

    /// Create a new deliberator with external judge validation.
    pub fn with_judge(
        thinking_provider: Arc<dyn Provider>,
        judge: ResponseJudge,
        config: DeliberationConfig,
    ) -> Self {
        Self {
            thinking_provider,
            judge: Some(judge),
            config,
        }
    }

    /// Perform multi-stage deliberation on a task.
    pub async fn deliberate(
        &self,
        task: &str,
        messages: Vec<Message>,
        available_tools: &[ToolInfo],
        tool_results: Option<&[ToolExecutionResult]>,
    ) -> crate::Result<DeliberationResult> {
        let mut critique = None;
        let mut iterations: u8 = 0;
        let mut confidence: f32 = 1.0;

        // Stage 1: Initial response with thinking
        let mut current_response = if self.config.enable_thinking {
            self.generate_with_thinking(&messages).await?
        } else {
            let content = self.thinking_provider.complete(messages.clone()).await?;
            ProviderResponse {
                content,
                reasoning_content: None,
                tool_calls: vec![],
                finish_reason: None,
            }
        };

        let thinking = current_response.reasoning_content.clone();
        iterations += 1;

        // Stage 2: Self-critique (if enabled)
        if self.config.enable_critique && iterations < self.config.max_rounds {
            let critique_result = self.generate_critique(task, &current_response).await?;

            if let Some(crit) = critique_result {
                // Check if critique found issues
                if self.critique_has_issues(&crit) {
                    confidence -= 0.2;

                    // Refine based on critique
                    current_response = self
                        .refine_response(task, &messages, &current_response, &crit)
                        .await?;
                    iterations += 1;
                }
                critique = Some(crit);
            }
        }

        // Stage 3: External judge validation (if enabled)
        if self.config.enable_judge
            && iterations < self.config.max_rounds
            && let Some(ref judge) = self.judge
        {
            let judgment = judge
                .evaluate(task, &current_response, available_tools, tool_results)
                .await?;

            if !judgment.passed {
                confidence -= 0.3;

                // Refine based on judgment
                current_response = self
                    .refine_with_judgment(task, &messages, &current_response, &judgment)
                    .await?;
                iterations += 1;
            }
        }

        // Clamp confidence to valid range
        confidence = confidence.clamp(0.0, 1.0);

        Ok(DeliberationResult {
            thinking,
            critique,
            final_response: current_response.content,
            iterations,
            confidence,
        })
    }

    /// Generate a response with thinking/reasoning enabled.
    async fn generate_with_thinking(
        &self,
        messages: &[Message],
    ) -> crate::Result<ProviderResponse> {
        // Add a system prompt to encourage reasoning
        let mut enhanced_messages = vec![Message {
            role: Role::System,
            content: "Think through this problem step-by-step before providing your answer. \
                      Consider multiple approaches and potential issues."
                .to_string(),
            images: None,
        }];
        enhanced_messages.extend(messages.iter().cloned());

        let content = self
            .thinking_provider
            .complete(enhanced_messages)
            .await
            .map_err(|e| crate::Error::ModelExecution(e.to_string()))?;

        // TODO: When streaming or using reasoning models, extract reasoning_content
        Ok(ProviderResponse {
            content,
            reasoning_content: None, // Will be populated by reasoning-capable providers
            tool_calls: vec![],
            finish_reason: None,
        })
    }

    /// Generate a critique of the current response.
    async fn generate_critique(
        &self,
        task: &str,
        response: &ProviderResponse,
    ) -> crate::Result<Option<String>> {
        let critique_prompt = format!(
            r#"Critique this response for the task: "{}"

Response to critique:
{}

Check for:
1. Accuracy and completeness - Does it fully address the task?
2. Tool error handling - If tools failed, was the error acknowledged?
3. Logical consistency - Are there any contradictions?
4. Missing information - What key points were omitted?

If no significant issues found, respond with exactly: PASS
Otherwise, list the specific issues found."#,
            task,
            response.content.chars().take(1000).collect::<String>()
        );

        let critique_response = self
            .thinking_provider
            .complete(vec![Message {
                role: Role::User,
                content: critique_prompt,
                images: None,
            }])
            .await
            .map_err(|e| crate::Error::ModelExecution(e.to_string()))?;

        if critique_response.to_uppercase().contains("PASS") {
            Ok(None)
        } else {
            Ok(Some(critique_response))
        }
    }

    /// Check if the critique indicates issues that need addressing.
    fn critique_has_issues(&self, critique: &str) -> bool {
        let critique_lower = critique.to_lowercase();

        // If it says PASS, no issues
        if critique_lower.contains("pass") && !critique_lower.contains("password") {
            return false;
        }

        // Look for issue indicators
        let issue_indicators = [
            "error",
            "incorrect",
            "wrong",
            "missing",
            "incomplete",
            "issue",
            "problem",
            "should",
            "must",
            "needs to",
            "failed to",
            "doesn't",
            "does not",
        ];

        issue_indicators
            .iter()
            .any(|ind| critique_lower.contains(ind))
    }

    /// Refine the response based on critique feedback.
    async fn refine_response(
        &self,
        task: &str,
        original_messages: &[Message],
        current_response: &ProviderResponse,
        critique: &str,
    ) -> crate::Result<ProviderResponse> {
        let mut messages = original_messages.to_vec();

        // Add the assistant's previous response
        messages.push(Message {
            role: Role::Assistant,
            content: current_response.content.clone(),
            images: None,
        });

        // Add the critique as user feedback
        messages.push(Message {
            role: Role::User,
            content: format!(
                "The previous response had these issues:\n{critique}\n\n\
                 Please provide an improved response that addresses these concerns for the task: {task}"
            ),
            images: None,
        });

        let refined = self
            .thinking_provider
            .complete(messages)
            .await
            .map_err(|e| crate::Error::ModelExecution(e.to_string()))?;

        Ok(ProviderResponse {
            content: refined,
            reasoning_content: None,
            tool_calls: vec![],
            finish_reason: None,
        })
    }

    /// Refine the response based on judge feedback.
    async fn refine_with_judgment(
        &self,
        task: &str,
        original_messages: &[Message],
        current_response: &ProviderResponse,
        judgment: &JudgmentResult,
    ) -> crate::Result<ProviderResponse> {
        let mut messages = original_messages.to_vec();

        // Add the assistant's previous response
        messages.push(Message {
            role: Role::Assistant,
            content: current_response.content.clone(),
            images: None,
        });

        // Add the judgment feedback
        let issue_description = match judgment.issue_type {
            crate::judge::IssueType::ToolErrorIgnored => {
                "You ignored or contradicted a tool execution error. \
                 When a tool fails, you must acknowledge the error and explain what went wrong."
            }
            crate::judge::IssueType::MissingToolUse => {
                "You should have used a tool to complete this task but didn't."
            }
            crate::judge::IssueType::HallucinatedTool => {
                "You tried to use a tool that doesn't exist."
            }
            crate::judge::IssueType::Refusal => {
                "You refused to complete the task when you should have attempted it."
            }
            _ => "The response had quality issues.",
        };

        messages.push(Message {
            role: Role::User,
            content: format!(
                "Quality check failed: {}\n\
                 Reason: {}\n\n\
                 Please provide a corrected response for the task: {}",
                issue_description,
                judgment.reason.as_deref().unwrap_or("No specific reason"),
                task
            ),
            images: None,
        });

        let refined = self
            .thinking_provider
            .complete(messages)
            .await
            .map_err(|e| crate::Error::ModelExecution(e.to_string()))?;

        Ok(ProviderResponse {
            content: refined,
            reasoning_content: None,
            tool_calls: vec![],
            finish_reason: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct MockDeliberationProvider {
        responses: std::sync::Mutex<Vec<String>>,
    }

    impl MockDeliberationProvider {
        fn new(responses: Vec<String>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses),
            }
        }
    }

    #[async_trait::async_trait]
    impl Provider for MockDeliberationProvider {
        async fn complete(&self, _messages: Vec<Message>) -> arkavo_llm::Result<String> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                Ok("Default response".to_string())
            } else {
                Ok(responses.remove(0))
            }
        }

        async fn complete_with_options(
            &self,
            messages: Vec<Message>,
            _max_tokens: Option<usize>,
        ) -> arkavo_llm::Result<String> {
            self.complete(messages).await
        }

        async fn stream(
            &self,
            _messages: Vec<Message>,
        ) -> arkavo_llm::Result<
            Box<
                dyn tokio_stream::Stream<Item = arkavo_llm::Result<arkavo_llm::StreamResponse>>
                    + Send
                    + Unpin,
            >,
        > {
            unimplemented!()
        }

        fn name(&self) -> &str {
            "mock_deliberation"
        }
    }

    fn create_test_tool_info(name: &str) -> ToolInfo {
        ToolInfo {
            name: name.to_string(),
            category: "Test".to_string(),
            description: "Test tool".to_string(),
            schema: json!({
                "type": "object",
                "properties": {},
            }),
        }
    }

    #[tokio::test]
    async fn test_deliberation_passes_first_time() {
        let provider = Arc::new(MockDeliberationProvider::new(vec![
            "Good response to the task.".to_string(),
            "PASS".to_string(), // Critique passes
        ]));

        let deliberator = Deliberator::new(
            provider,
            DeliberationConfig {
                enable_thinking: true,
                enable_critique: true,
                enable_judge: false,
                max_rounds: 3,
            },
        );

        let result = deliberator
            .deliberate(
                "Test task",
                vec![Message {
                    role: Role::User,
                    content: "Do the task".to_string(),
                    images: None,
                }],
                &[create_test_tool_info("test_tool")],
                None,
            )
            .await
            .unwrap();

        assert_eq!(result.final_response, "Good response to the task.");
        assert_eq!(result.iterations, 1);
        assert!(result.confidence > 0.9);
        assert!(result.critique.is_none());
    }

    #[tokio::test]
    async fn test_deliberation_with_refinement() {
        let provider = Arc::new(MockDeliberationProvider::new(vec![
            "Initial incomplete response.".to_string(),
            "Issue: Missing key information about X.".to_string(), // Critique finds issues
            "Improved response with all information.".to_string(), // Refined response
        ]));

        let deliberator = Deliberator::new(
            provider,
            DeliberationConfig {
                enable_thinking: true,
                enable_critique: true,
                enable_judge: false,
                max_rounds: 3,
            },
        );

        let result = deliberator
            .deliberate(
                "Test task",
                vec![Message {
                    role: Role::User,
                    content: "Do the task".to_string(),
                    images: None,
                }],
                &[],
                None,
            )
            .await
            .unwrap();

        assert_eq!(
            result.final_response,
            "Improved response with all information."
        );
        assert_eq!(result.iterations, 2);
        assert!(result.confidence < 1.0); // Reduced due to critique issues
        assert!(result.critique.is_some());
    }

    #[tokio::test]
    async fn test_deliberation_max_rounds() {
        // Test that we don't exceed max_rounds
        let config = DeliberationConfig {
            enable_thinking: true,
            enable_critique: true,
            enable_judge: false,
            max_rounds: 2,
        };

        let provider = Arc::new(MockDeliberationProvider::new(vec![
            "Response 1".to_string(),
            "Issue found".to_string(), // Critique finds issues
            "Response 2".to_string(),
        ]));

        let deliberator = Deliberator::new(provider, config);

        let result = deliberator
            .deliberate(
                "Test task",
                vec![Message {
                    role: Role::User,
                    content: "Do the task".to_string(),
                    images: None,
                }],
                &[],
                None,
            )
            .await
            .unwrap();

        assert!(result.iterations <= 2);
    }

    #[tokio::test]
    async fn test_critique_has_issues_detection() {
        let provider = Arc::new(MockDeliberationProvider::new(vec![]));
        let deliberator = Deliberator::new(provider, DeliberationConfig::default());

        // Should detect issues
        assert!(deliberator.critique_has_issues("The response is incorrect"));
        assert!(deliberator.critique_has_issues("Missing important details"));
        assert!(deliberator.critique_has_issues("This should be fixed"));
        assert!(deliberator.critique_has_issues("There's an issue with the logic"));

        // Should not detect issues (PASS)
        assert!(!deliberator.critique_has_issues("PASS"));
        assert!(!deliberator.critique_has_issues("Everything looks good. PASS"));
    }
}
