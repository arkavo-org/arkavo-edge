use arkavo_llm::{Message, Provider, ProviderResponse, Role};
use arkavo_mcp_tools::ToolInfo;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum IssueType {
    HallucinatedTool,
    InvalidParams,
    Refusal,
    OffTopic,
    MissingToolUse, // LLM should have used a tool but didn't
    None,
}

impl std::fmt::Display for IssueType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IssueType::HallucinatedTool => write!(f, "hallucinated_tool"),
            IssueType::InvalidParams => write!(f, "invalid_params"),
            IssueType::Refusal => write!(f, "refusal"),
            IssueType::OffTopic => write!(f, "off_topic"),
            IssueType::MissingToolUse => write!(f, "missing_tool_use"),
            IssueType::None => write!(f, "none"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct JudgmentResult {
    pub passed: bool,
    pub reason: Option<String>,
    pub issue_type: IssueType,
    pub suggested_keywords: Vec<String>, // For MissingToolUse, keywords to search for tools
}

pub struct ResponseJudge {
    judge_provider: Arc<dyn Provider>,
}

impl ResponseJudge {
    #[cfg(feature = "llama-cpp")]
    pub fn new_gemma_270m() -> crate::Result<Self> {
        let model_path = std::env::var("ARKAVO_GEMMA_270M_PATH").unwrap_or_else(|_| {
            // Try common locations for Gemma-3 270M GGUF model
            let hf_path = std::env::var("HF_HOME")
                .or_else(|_| std::env::var("HOME").map(|h| format!("{h}/.cache/huggingface")))
                .unwrap_or_else(|_| "~/.cache/huggingface".to_string());

            // Check multiple possible locations
            let candidates = vec![
                format!("{hf_path}/hub/models--unsloth--gemma-3-270m-it-GGUF/snapshots/*/gemma-3-270m-it-Q4_K_M.gguf"),
                format!("{hf_path}/hub/models--bartowski--gemma-3-270m-it-GGUF/snapshots/*/gemma-3-270m-it-Q4_K_M.gguf"),
                "models/gemma-3-270m-it.gguf".to_string(),
                "/Volumes/SSD/huggingface/hub/models--unsloth--gemma-3-270m-it-GGUF/snapshots/*/gemma-3-270m-it-Q4_K_M.gguf".to_string(),
            ];

            // Return first existing path or default
            for candidate in candidates {
                if let Ok(entries) = glob::glob(&candidate)
                    && let Some(Ok(path)) = entries.into_iter().next()
                {
                    return path.to_string_lossy().to_string();
                }
            }

            "models/gemma-3-270m-it.gguf".to_string()
        });

        let provider = arkavo_llm::LlamaCppProvider::new("gemma-3-270m-it".to_string(), model_path)
            .map_err(|e| {
                crate::Error::ModelExecution(format!("Failed to create judge provider: {e}"))
            })?;

        Ok(Self {
            judge_provider: Arc::new(provider),
        })
    }

    #[cfg(feature = "llama-cpp")]
    pub fn new_gemma_4b() -> crate::Result<Self> {
        let model_path = std::env::var("ARKAVO_GEMMA_4B_PATH").unwrap_or_else(|_| {
            // Try common locations for Gemma 4B GGUF model
            let hf_path = std::env::var("HF_HOME")
                .or_else(|_| std::env::var("HOME").map(|h| format!("{h}/.cache/huggingface")))
                .unwrap_or_else(|_| "~/.cache/huggingface".to_string());

            // Check multiple possible locations
            let candidates = vec![
                format!("{hf_path}/hub/models--unsloth--gemma-3-4b-it-GGUF/snapshots/*/gemma-3-4b-it-Q4_K_M.gguf"),
                "models/gemma-3-4b-it.gguf".to_string(),
                "/Volumes/SSD/huggingface/hub/models--unsloth--gemma-3-4b-it-GGUF/snapshots/*/gemma-3-4b-it-Q4_K_M.gguf".to_string(),
            ];

            // Return first existing path or default
            for candidate in candidates {
                if let Ok(entries) = glob::glob(&candidate)
                    && let Some(Ok(path)) = entries.into_iter().next()
                {
                    return path.to_string_lossy().to_string();
                }
            }

            "models/gemma-3-4b-it.gguf".to_string()
        });

        let provider = arkavo_llm::LlamaCppProvider::new("gemma-3-4b-it".to_string(), model_path)
            .map_err(|e| {
            crate::Error::ModelExecution(format!("Failed to create judge provider: {e}"))
        })?;

        Ok(Self {
            judge_provider: Arc::new(provider),
        })
    }

    #[cfg(not(feature = "llama-cpp"))]
    pub fn new_gemma_270m() -> crate::Result<Self> {
        Err(crate::Error::Config(
            "Judge requires llama-cpp feature to be enabled".to_string(),
        ))
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
        // Step 1: Run heuristics (instant, free, catches 50% of cases)
        let heuristic_result = self.heuristic_check(original_prompt, response, available_tools);

        // If heuristics found a clear FAIL, trust it (heuristics caught this one!)
        if !heuristic_result.passed {
            tracing::info!(
                "Heuristic judge: FAIL - {}",
                heuristic_result.reason.as_deref().unwrap_or("")
            );
            return Ok(heuristic_result);
        }

        // If tools were used, it's a clear pass
        if !response.tool_calls.is_empty() {
            tracing::debug!("Heuristic judge: PASS (tools used)");
            return Ok(heuristic_result);
        }

        // Check if there's any sign of refusal that needs LLM validation
        let response_lower = response.content.to_lowercase();
        let has_refusal_hint = response_lower.contains("don't")
            || response_lower.contains("can't")
            || response_lower.contains("cannot")
            || response_lower.contains("unable")
            || response_lower.contains("not able");

        // If heuristics say PASS but we see refusal hints, double-check with 270M judge
        if !has_refusal_hint {
            tracing::debug!("Heuristic judge: PASS (no refusal hints)");
            return Ok(heuristic_result);
        }

        // Step 2: Refusal hint detected - use 270M LLM judge for validation
        let judge_prompt = self.build_judge_prompt(original_prompt, response, available_tools);

        tracing::info!(
            "=== JUDGE PROMPT START ===\n{}\n=== JUDGE PROMPT END ===",
            judge_prompt
        );

        let judgment_text = self
            .judge_provider
            .complete(vec![Message {
                role: Role::User,
                content: judge_prompt,
                images: None,
            }])
            .await
            .map_err(|e| {
                tracing::warn!("Judge LLM failed, falling back to heuristics: {}", e);
                // Fall back to heuristic result if LLM fails
                return crate::Error::ModelExecution(format!("Judge evaluation failed: {e}"));
            })?;

        tracing::debug!("Judge raw output:\n{}", judgment_text);

        // Try to parse LLM output, fall back to heuristics if parsing fails
        self.parse_judgment(&judgment_text).or_else(|e| {
            tracing::warn!("Judge parsing failed, using heuristics: {}", e);
            Ok(heuristic_result)
        })
    }

    fn heuristic_check(
        &self,
        original_prompt: &str,
        response: &ProviderResponse,
        available_tools: &[ToolInfo],
    ) -> JudgmentResult {
        let response_lower = response.content.to_lowercase();
        let prompt_lower = original_prompt.to_lowercase();

        // Check if AI refused to answer
        let has_refusal = response_lower.contains("i don't know")
            || response_lower.contains("i can't")
            || response_lower.contains("i cannot")
            || response_lower.contains("don't have access")
            || response_lower.contains("i'm not able")
            || response_lower.contains("unable to");

        // Check if AI used any tools
        let used_tools = !response.tool_calls.is_empty();

        // If AI used tools or didn't refuse, it's a pass
        if used_tools || !has_refusal {
            return JudgmentResult {
                passed: true,
                reason: None,
                issue_type: IssueType::None,
                suggested_keywords: Vec::new(),
            };
        }

        // AI refused and didn't use tools - check if relevant tools exist
        let suggested_keywords = self.extract_keywords_from_prompt(&prompt_lower, available_tools);

        tracing::debug!(
            "Heuristic check: prompt='{}', has_refusal={}, available_tools={}, suggested_keywords={:?}",
            prompt_lower,
            has_refusal,
            available_tools.len(),
            suggested_keywords
        );

        if suggested_keywords.is_empty() {
            // No relevant tools found, refusal is acceptable
            JudgmentResult {
                passed: true,
                reason: None,
                issue_type: IssueType::None,
                suggested_keywords: Vec::new(),
            }
        } else {
            // Relevant tools exist but weren't used
            JudgmentResult {
                passed: false,
                reason: Some(format!("SUGGEST: {}", suggested_keywords.join(", "))),
                issue_type: IssueType::MissingToolUse,
                suggested_keywords,
            }
        }
    }

    fn extract_keywords_from_prompt(
        &self,
        prompt: &str,
        available_tools: &[ToolInfo],
    ) -> Vec<String> {
        let mut keywords = Vec::new();

        // Pattern matching for common requests
        if prompt.contains("time") || prompt.contains("clock") || prompt.contains("date") {
            // Check if time-related tool exists
            for tool in available_tools {
                let tool_desc = format!("{} {}", tool.name, tool.description).to_lowercase();
                if tool_desc.contains("time")
                    || tool_desc.contains("clock")
                    || tool_desc.contains("date")
                {
                    keywords.extend(vec!["time".to_string(), "clock".to_string()]);
                    break;
                }
            }
        }

        if prompt.contains("search")
            || prompt.contains("find")
            || prompt.contains("lookup")
            || prompt.contains("google")
        {
            for tool in available_tools {
                let tool_desc = format!("{} {}", tool.name, tool.description).to_lowercase();
                if tool_desc.contains("search")
                    || tool_desc.contains("web")
                    || tool_desc.contains("google")
                {
                    keywords.extend(vec!["search".to_string(), "web".to_string()]);
                    break;
                }
            }
        }

        if prompt.contains("file")
            || prompt.contains("read")
            || prompt.contains("write")
            || prompt.contains("directory")
        {
            for tool in available_tools {
                let tool_desc = format!("{} {}", tool.name, tool.description).to_lowercase();
                if tool_desc.contains("file")
                    || tool_desc.contains("filesystem")
                    || tool_desc.contains("directory")
                {
                    keywords.extend(vec!["file".to_string(), "filesystem".to_string()]);
                    break;
                }
            }
        }

        if prompt.contains("run")
            || prompt.contains("execute")
            || prompt.contains("command")
            || prompt.contains("shell")
        {
            for tool in available_tools {
                let tool_desc = format!("{} {}", tool.name, tool.description).to_lowercase();
                if tool_desc.contains("bash")
                    || tool_desc.contains("shell")
                    || tool_desc.contains("command")
                    || tool_desc.contains("execute")
                {
                    keywords.extend(vec!["bash".to_string(), "shell".to_string()]);
                    break;
                }
            }
        }

        // Remove duplicates
        keywords.sort();
        keywords.dedup();
        keywords
    }

    fn build_judge_prompt(
        &self,
        original_prompt: &str,
        response: &ProviderResponse,
        available_tools: &[ToolInfo],
    ) -> String {
        let tools_list = available_tools
            .iter()
            .take(10) // Limit to first 10 tools to keep prompt short
            .map(|t| format!("- {} ({})", t.name, t.description))
            .collect::<Vec<_>>()
            .join("\n");

        let tool_calls_text = if response.tool_calls.is_empty() {
            "None".to_string()
        } else {
            response
                .tool_calls
                .iter()
                .map(|call| call.tool_name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        };

        format!(
            r#"You are judging if an AI should have used a tool.

User asked: "{}"

AI responded: "{}"

Tools AI used: {}

Available tools:
{}

Question: Did the AI refuse when it should have used a tool?

If AI said "I don't know" or "I can't" but a relevant tool exists, answer:
VERDICT: FAIL
REASON: SUGGEST: keyword1, keyword2
ISSUE: missing_tool_use

If AI used a tool or didn't refuse, answer:
VERDICT: PASS
ISSUE: none

Your answer:"#,
            original_prompt,
            // Truncate long responses but keep key refusal phrases
            if response.content.len() > 200 {
                format!("{}...", &response.content[..200])
            } else {
                response.content.clone()
            },
            tool_calls_text,
            tools_list
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
                s if s.contains("missing") => IssueType::MissingToolUse,
                s if s.contains("off") => IssueType::OffTopic,
                _ => IssueType::None,
            })
            .unwrap_or(IssueType::None);

        // Extract suggested keywords from REASON if present (format: "SUGGEST: keyword1, keyword2")
        let suggested_keywords = if issue_type == IssueType::MissingToolUse {
            reason
                .as_ref()
                .and_then(|r| {
                    r.to_uppercase().find("SUGGEST:").map(|pos| &r[pos + 8..]) // Skip "SUGGEST:"
                })
                .map(|keywords_str| {
                    keywords_str
                        .split(',')
                        .map(|k| k.trim().to_lowercase())
                        .filter(|k| !k.is_empty())
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        Ok(JudgmentResult {
            passed,
            reason,
            issue_type,
            suggested_keywords,
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
