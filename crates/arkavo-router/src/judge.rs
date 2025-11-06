use arkavo_llm::{Message, Provider, ProviderResponse, Role};
use arkavo_mcp_tools::ToolInfo;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum IssueType {
    HallucinatedTool,
    InvalidParams,
    Refusal,
    OffTopic,
    None,
}

impl std::fmt::Display for IssueType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IssueType::HallucinatedTool => write!(f, "hallucinated_tool"),
            IssueType::InvalidParams => write!(f, "invalid_params"),
            IssueType::Refusal => write!(f, "refusal"),
            IssueType::OffTopic => write!(f, "off_topic"),
            IssueType::None => write!(f, "none"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct JudgmentResult {
    pub passed: bool,
    pub reason: Option<String>,
    pub issue_type: IssueType,
}

pub struct ResponseJudge {
    judge_provider: Arc<dyn Provider>,
}

impl ResponseJudge {
    #[cfg(feature = "llama-cpp")]
    pub fn new_gemma_4b() -> crate::Result<Self> {
        let model_path = std::env::var("ARKAVO_GEMMA_4B_PATH")
            .unwrap_or_else(|_| "models/gemma-3-4b-it.gguf".to_string());

        let provider = arkavo_llm::LlamaCppProvider::new("gemma-3-4b-it".to_string(), model_path)
            .map_err(|e| {
            crate::Error::ModelExecution(format!("Failed to create judge provider: {e}"))
        })?;

        Ok(Self {
            judge_provider: Arc::new(provider),
        })
    }

    #[cfg(not(feature = "llama-cpp"))]
    pub fn new_gemma_4b() -> crate::Result<Self> {
        Err(crate::Error::Config(
            "Judge requires llama-cpp feature to be enabled".to_string(),
        ))
    }

    pub fn new(provider: Arc<dyn Provider>) -> Self {
        Self {
            judge_provider: provider,
        }
    }

    pub async fn evaluate(
        &self,
        original_prompt: &str,
        response: &ProviderResponse,
        available_tools: &[ToolInfo],
    ) -> crate::Result<JudgmentResult> {
        let prompt = self.build_judge_prompt(original_prompt, response, available_tools);

        let judgment_text = self
            .judge_provider
            .complete(vec![Message {
                role: Role::User,
                content: prompt,
                images: None,
            }])
            .await
            .map_err(|e| crate::Error::ModelExecution(format!("Judge evaluation failed: {e}")))?;

        self.parse_judgment(&judgment_text)
    }

    fn build_judge_prompt(
        &self,
        original_prompt: &str,
        response: &ProviderResponse,
        available_tools: &[ToolInfo],
    ) -> String {
        let tools_list = available_tools
            .iter()
            .map(|t| format!("- {} - {}", t.name, t.description))
            .collect::<Vec<_>>()
            .join("\n");

        let tool_calls_text = if response.tool_calls.is_empty() {
            "[No tool calls]".to_string()
        } else {
            response
                .tool_calls
                .iter()
                .map(|call| {
                    format!(
                        "- {} with args: {}",
                        call.tool_name,
                        serde_json::to_string(&call.arguments).unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        format!(
            r#"You are a quality judge for AI-generated responses involving tool calls.

AVAILABLE TOOLS:
{}

USER REQUEST:
{}

AI RESPONSE:
Content: {}
Tool Calls:
{}

TASK: Evaluate if the response is valid and appropriate.

Check for:
1. Tool hallucination - Did the AI call tools that don't exist?
2. Invalid parameters - Are tool arguments valid per schema?
3. Refusal - Did the AI refuse instead of using tools?
4. Off-topic - Is the response relevant to the request?

Reply in this exact format:
VERDICT: PASS or FAIL
REASON: Brief explanation (if FAIL)
ISSUE: hallucinated_tool | invalid_params | refusal | off_topic | none"#,
            tools_list, original_prompt, response.content, tool_calls_text
        )
    }

    fn parse_judgment(&self, judgment_text: &str) -> crate::Result<JudgmentResult> {
        let lines: Vec<&str> = judgment_text.lines().collect();

        let verdict_line = lines
            .iter()
            .find(|line| line.to_uppercase().contains("VERDICT:"))
            .ok_or_else(|| {
                crate::Error::Internal("Judge response missing VERDICT line".to_string())
            })?;

        let passed = verdict_line.to_uppercase().contains("PASS");

        let reason = if !passed {
            lines
                .iter()
                .find(|line| line.to_uppercase().contains("REASON:"))
                .and_then(|line| line.split(':').nth(1))
                .map(|s| s.trim().to_string())
        } else {
            None
        };

        let issue_type = lines
            .iter()
            .find(|line| line.to_uppercase().contains("ISSUE:"))
            .and_then(|line| line.split(':').nth(1))
            .map(|s| s.trim().to_lowercase())
            .map(|issue_str| match issue_str.as_str() {
                s if s.contains("hallucinated") => IssueType::HallucinatedTool,
                s if s.contains("invalid") => IssueType::InvalidParams,
                s if s.contains("refusal") => IssueType::Refusal,
                s if s.contains("off") => IssueType::OffTopic,
                _ => IssueType::None,
            })
            .unwrap_or(IssueType::None);

        Ok(JudgmentResult {
            passed,
            reason,
            issue_type,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_llm::tool_parser::ParsedToolCall;
    use serde_json::json;

    struct MockJudgeProvider {
        response: String,
    }

    #[async_trait::async_trait]
    impl Provider for MockJudgeProvider {
        async fn complete(&self, _messages: Vec<Message>) -> arkavo_llm::Result<String> {
            Ok(self.response.clone())
        }

        async fn complete_with_options(
            &self,
            _messages: Vec<Message>,
            _max_tokens: Option<usize>,
        ) -> arkavo_llm::Result<String> {
            Ok(self.response.clone())
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
            "mock_judge"
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
    async fn test_judgment_pass() {
        let provider = Arc::new(MockJudgeProvider {
            response: "VERDICT: PASS\nREASON: Good response\nISSUE: none".to_string(),
        });

        let judge = ResponseJudge::new(provider);
        let tools = vec![create_test_tool_info("search")];

        let response = ProviderResponse {
            content: "Searching...".to_string(),
            tool_calls: vec![ParsedToolCall {
                tool_name: "search".to_string(),
                arguments: json!({"query": "test"}),
                call_id: None,
            }],
            finish_reason: None,
        };

        let result = judge
            .evaluate("Find test", &response, &tools)
            .await
            .unwrap();
        assert!(result.passed);
        assert_eq!(result.issue_type, IssueType::None);
    }

    #[tokio::test]
    async fn test_judgment_fail_hallucinated() {
        let provider = Arc::new(MockJudgeProvider {
            response: "VERDICT: FAIL\nREASON: Tool does not exist\nISSUE: hallucinated_tool"
                .to_string(),
        });

        let judge = ResponseJudge::new(provider);
        let tools = vec![create_test_tool_info("search")];

        let response = ProviderResponse {
            content: "Using tool...".to_string(),
            tool_calls: vec![ParsedToolCall {
                tool_name: "fake_tool".to_string(),
                arguments: json!({}),
                call_id: None,
            }],
            finish_reason: None,
        };

        let result = judge.evaluate("Test", &response, &tools).await.unwrap();
        assert!(!result.passed);
        assert_eq!(result.issue_type, IssueType::HallucinatedTool);
        assert!(result.reason.is_some());
    }

    #[tokio::test]
    async fn test_judgment_fail_refusal() {
        let provider = Arc::new(MockJudgeProvider {
            response: "VERDICT: FAIL\nREASON: AI refused to use tools\nISSUE: refusal".to_string(),
        });

        let judge = ResponseJudge::new(provider);
        let tools = vec![create_test_tool_info("search")];

        let response = ProviderResponse {
            content: "I do not have access to any tools".to_string(),
            tool_calls: vec![],
            finish_reason: None,
        };

        let result = judge
            .evaluate("Use tools", &response, &tools)
            .await
            .unwrap();
        assert!(!result.passed);
        assert_eq!(result.issue_type, IssueType::Refusal);
    }
}
