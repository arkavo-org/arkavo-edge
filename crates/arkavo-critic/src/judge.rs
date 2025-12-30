//! LLM-based response quality judgment
//!
//! Uses a small local model to evaluate response quality, detecting:
//! - Hallucinated tools
//! - Invalid parameters
//! - Refusals when tools should have been used
//! - Tool error acknowledgment failures

use arkavo_llm::{Message, Provider, ProviderResponse, Role, tool_executor::ToolExecutionResult};
use arkavo_mcp_tools::ToolInfo;
use std::sync::Arc;

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum IssueType {
    HallucinatedTool,
    InvalidParams,
    Refusal,
    OffTopic,
    MissingToolUse,
    ToolErrorIgnored,
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
            IssueType::ToolErrorIgnored => write!(f, "tool_error_ignored"),
            IssueType::None => write!(f, "none"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct JudgmentResult {
    pub passed: bool,
    pub reason: Option<String>,
    pub issue_type: IssueType,
    pub suggested_keywords: Vec<String>,
}

pub struct ResponseJudge {
    judge_provider: Arc<dyn Provider>,
}

impl ResponseJudge {
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
        tool_results: Option<&[ToolExecutionResult]>,
    ) -> Result<JudgmentResult> {
        // Step 0: Check if LLM properly acknowledged tool execution errors
        if let Some(results) = tool_results
            && let Some(error_judgment) = self.check_tool_error_acknowledgment(response, results)
        {
            tracing::info!(
                "Tool error acknowledgment check: FAIL - {}",
                error_judgment.reason.as_deref().unwrap_or("")
            );
            return Ok(error_judgment);
        }

        // Step 1: Run heuristics (instant, free, catches 50% of cases)
        let heuristic_result = self.heuristic_check(original_prompt, response, available_tools);

        // If heuristics found a clear FAIL, trust it
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
            || response_lower.contains("do not")
            || response_lower.contains("can't")
            || response_lower.contains("cannot")
            || response_lower.contains("unable")
            || response_lower.contains("not able")
            || response_lower.contains("no access")
            || response_lower.contains("don't have access")
            || response_lower.contains("do not have access");

        // If heuristics say PASS but we see refusal hints, double-check with LLM judge
        if !has_refusal_hint {
            tracing::debug!("Heuristic judge: PASS (no refusal hints)");
            return Ok(heuristic_result);
        }

        // Step 2: Refusal hint detected - use LLM judge for validation
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
                Error::Judge(format!("Judge evaluation failed: {e}"))
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
            || response_lower.contains("i do not know")
            || response_lower.contains("i can't")
            || response_lower.contains("i cannot")
            || response_lower.contains("don't have access")
            || response_lower.contains("do not have access")
            || response_lower.contains("i'm not able")
            || response_lower.contains("unable to");

        // Check for hallucinated tools (tools that don't exist)
        let available_tool_names: std::collections::HashSet<&str> =
            available_tools.iter().map(|t| t.name.as_str()).collect();

        for call in &response.tool_calls {
            if !available_tool_names.contains(call.tool_name.as_str()) {
                return JudgmentResult {
                    passed: false,
                    reason: Some(format!("Tool '{}' does not exist", call.tool_name)),
                    issue_type: IssueType::HallucinatedTool,
                    suggested_keywords: Vec::new(),
                };
            }
        }

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

    fn check_tool_error_acknowledgment(
        &self,
        response: &ProviderResponse,
        tool_results: &[ToolExecutionResult],
    ) -> Option<JudgmentResult> {
        // Find failed tool executions
        let failed_tools: Vec<_> = tool_results.iter().filter(|r| !r.success).collect();

        if failed_tools.is_empty() {
            return None;
        }

        let response_lower = response.content.to_lowercase();

        // Success-claiming phrases that would contradict a tool failure
        let success_claims = [
            "successfully",
            "completed",
            "done",
            "sent it",
            "executed",
            "finished",
            "accomplished",
            "worked",
        ];

        // Error acknowledgment phrases
        let error_indicators = [
            "error",
            "failed",
            "couldn't",
            "could not",
            "unable",
            "problem",
            "not found",
            "doesn't exist",
            "does not exist",
            "unavailable",
            "issue",
        ];

        for failed in &failed_tools {
            let tool_name_lower = failed.tool_name.to_lowercase();
            let error_msg = failed.error.as_deref().unwrap_or("Unknown error");

            // Check if LLM claims success for this tool (contradiction)
            let claims_success = success_claims.iter().any(|claim| {
                let has_claim = response_lower.contains(claim);
                // Check if success claim is near the tool name
                let near_tool = response_lower
                    .find(&tool_name_lower)
                    .and_then(|tool_pos| {
                        response_lower.find(claim).map(|claim_pos| {
                            // Within 100 chars of each other
                            tool_pos.abs_diff(claim_pos) < 100
                        })
                    })
                    .unwrap_or(false);
                has_claim && near_tool
            });

            if claims_success {
                return Some(JudgmentResult {
                    passed: false,
                    reason: Some(format!(
                        "LLM claimed success for '{}' but tool returned error: {}",
                        failed.tool_name, error_msg
                    )),
                    issue_type: IssueType::ToolErrorIgnored,
                    suggested_keywords: Vec::new(),
                });
            }

            // Strict mode: Check if error was completely ignored (no mention at all)
            let mentions_error = error_indicators
                .iter()
                .any(|ind| response_lower.contains(ind));

            if !mentions_error {
                return Some(JudgmentResult {
                    passed: false,
                    reason: Some(format!(
                        "LLM did not acknowledge tool error for '{}': {}",
                        failed.tool_name, error_msg
                    )),
                    issue_type: IssueType::ToolErrorIgnored,
                    suggested_keywords: Vec::new(),
                });
            }
        }

        None
    }

    fn extract_keywords_from_prompt(
        &self,
        prompt: &str,
        available_tools: &[ToolInfo],
    ) -> Vec<String> {
        let mut keywords = Vec::new();

        // Pattern matching for common requests
        if prompt.contains("time") || prompt.contains("clock") || prompt.contains("date") {
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

        if prompt.contains("git")
            || prompt.contains("branch")
            || prompt.contains("commit")
            || prompt.contains("status")
            || prompt.contains("diff")
            || prompt.contains("log")
            || prompt.contains("push")
            || prompt.contains("pull")
        {
            for tool in available_tools {
                let tool_desc = format!("{} {}", tool.name, tool.description).to_lowercase();
                if tool_desc.contains("git")
                    || tool_desc.contains("branch")
                    || tool_desc.contains("commit")
                    || tool_desc.contains("repository")
                {
                    keywords.extend(vec!["git".to_string(), "branch".to_string()]);
                    break;
                }
            }
        }

        if prompt.contains("pr ")
            || prompt.contains("pull request")
            || prompt.contains("github pr")
            || prompt.contains("create pr")
            || prompt.contains("merge pr")
        {
            for tool in available_tools {
                let tool_desc = format!("{} {}", tool.name, tool.description).to_lowercase();
                if tool_desc.contains("pull request")
                    || tool_desc.contains("pr")
                    || tool_desc.contains("github")
                {
                    keywords.extend(vec!["github".to_string(), "pr".to_string()]);
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
            .take(25)
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
            if response.content.len() > 200 {
                format!("{}...", &response.content[..200])
            } else {
                response.content.clone()
            },
            tool_calls_text,
            tools_list
        )
    }

    fn parse_judgment(&self, judgment_text: &str) -> Result<JudgmentResult> {
        let lines: Vec<&str> = judgment_text.lines().collect();

        let verdict_line = lines
            .iter()
            .find(|line| line.to_uppercase().contains("VERDICT:"))
            .ok_or_else(|| Error::Judge("Judge response missing VERDICT line".to_string()))?;

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
                s if s.contains("tool_error") || s.contains("ignored") => {
                    IssueType::ToolErrorIgnored
                }
                _ => IssueType::None,
            })
            .unwrap_or(IssueType::None);

        // Extract suggested keywords from REASON if present
        let suggested_keywords = if issue_type == IssueType::MissingToolUse {
            reason
                .as_ref()
                .and_then(|r| r.to_uppercase().find("SUGGEST:").map(|pos| &r[pos + 8..]))
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
            reasoning_content: None,
            tool_calls: vec![ParsedToolCall {
                tool_name: "search".to_string(),
                arguments: json!({"query": "test"}),
                call_id: None,
            }],
            finish_reason: None,
        };

        let result = judge
            .evaluate("Find test", &response, &tools, None)
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
            reasoning_content: None,
            tool_calls: vec![ParsedToolCall {
                tool_name: "fake_tool".to_string(),
                arguments: json!({}),
                call_id: None,
            }],
            finish_reason: None,
        };

        let result = judge
            .evaluate("Test", &response, &tools, None)
            .await
            .unwrap();
        assert!(!result.passed);
        assert_eq!(result.issue_type, IssueType::HallucinatedTool);
        assert!(result.reason.is_some());
    }

    #[tokio::test]
    async fn test_tool_error_ignored_claims_success() {
        let provider = Arc::new(MockJudgeProvider {
            response: "VERDICT: PASS".to_string(),
        });

        let judge = ResponseJudge::new(provider);
        let tools = vec![create_test_tool_info("send_task")];

        // LLM claims success when tool failed
        let response = ProviderResponse {
            content: "I used the send_task tool and it completed successfully.".to_string(),
            reasoning_content: None,
            tool_calls: vec![],
            finish_reason: None,
        };

        // Tool execution failed
        let tool_results = vec![ToolExecutionResult {
            tool_name: "send_task".to_string(),
            call_id: None,
            result: json!({"error": "Agent not found"}),
            success: false,
            error: Some("Agent 'test-agent' not found".to_string()),
        }];

        let result = judge
            .evaluate("Send task to agent", &response, &tools, Some(&tool_results))
            .await
            .unwrap();

        assert!(!result.passed);
        assert_eq!(result.issue_type, IssueType::ToolErrorIgnored);
        assert!(result.reason.as_ref().unwrap().contains("claimed success"));
    }
}
