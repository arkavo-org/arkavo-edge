//! LLM-based intent analyzer using a local model (Qwen 0.8B or Ministral 3B).

use arkavo_tasks::intent_analyzer::{IntentAnalysis, IntentAnalyzer};
use arkavo_tasks::task_planner::TaskPlanError;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::{debug, warn};

pub(super) struct LlmIntentAnalyzer {
    router: Arc<arkavo_router::Router>,
}

impl LlmIntentAnalyzer {
    pub(super) fn new(router: Arc<arkavo_router::Router>) -> Self {
        Self { router }
    }
}

#[async_trait]
impl IntentAnalyzer for LlmIntentAnalyzer {
    async fn analyze(&self, intent: &str) -> Result<IntentAnalysis, TaskPlanError> {
        let provider = self
            .router
            .get_provider(&arkavo_router::ModelChoice::LocalQwen3)
            .await
            .or_else(|_| {
                futures::executor::block_on(
                    self.router
                        .get_provider(&arkavo_router::ModelChoice::LocalMinistral3B),
                )
            })
            .map_err(|_| TaskPlanError::LlmAnalysisFailed("no local model available".into()))?;

        let system_prompt = r#"You are a task decomposition engine. Given a user intent, break it into 1-6 subtasks.
Respond with ONLY a JSON object in this exact format:
{
  "keywords": ["keyword1"],
  "entities": [],
  "subtask_specs": [
    {
      "task_type": "search|filter|analyze|summarize|generate|transform",
      "description": "What this subtask does",
      "required_capabilities": [],
      "depends_on": []
    }
  ]
}

Rules:
- depends_on contains 0-based indices of subtasks this one depends on
- Keep subtask count between 1 and 6
- Use clear, actionable descriptions
- Order subtasks logically"#;

        let messages = vec![
            arkavo_llm::Message {
                role: arkavo_llm::Role::System,
                content: system_prompt.to_string(),
                images: None,
            },
            arkavo_llm::Message {
                role: arkavo_llm::Role::User,
                content: intent.to_string(),
                images: None,
            },
        ];

        let response = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            provider.complete(messages),
        )
        .await
        .map_err(|_| TaskPlanError::LlmAnalysisFailed("timeout after 20s".into()))?
        .map_err(|e| TaskPlanError::LlmAnalysisFailed(format!("LLM error: {e}")))?;

        debug!(
            response_len = response.len(),
            "LLM intent analysis response"
        );

        parse_llm_response(&response)
    }
}

fn parse_llm_response(response: &str) -> Result<IntentAnalysis, TaskPlanError> {
    let start = response
        .find('{')
        .ok_or_else(|| TaskPlanError::LlmAnalysisFailed("no JSON object in response".into()))?;
    let end = response
        .rfind('}')
        .ok_or_else(|| TaskPlanError::LlmAnalysisFailed("no closing brace in response".into()))?;

    let json_str = &response[start..=end];

    let analysis: IntentAnalysis = serde_json::from_str(json_str).map_err(|e| {
        warn!(json = json_str, error = %e, "Failed to parse LLM intent JSON");
        TaskPlanError::LlmAnalysisFailed(format!("invalid JSON: {e}"))
    })?;

    // Validate subtask specs
    let spec_count = analysis.subtask_specs.len();
    for spec in &analysis.subtask_specs {
        for &dep_idx in &spec.depends_on {
            if dep_idx >= spec_count {
                return Err(TaskPlanError::LlmAnalysisFailed(format!(
                    "dependency index {dep_idx} out of range (only {spec_count} specs)"
                )));
            }
        }
    }

    Ok(analysis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_response() {
        let response = r#"Here is the decomposition:
{
  "keywords": ["search", "filter"],
  "entities": [],
  "subtask_specs": [
    {
      "task_type": "search",
      "description": "Search for X",
      "required_capabilities": ["web_search"],
      "depends_on": []
    },
    {
      "task_type": "filter",
      "description": "Filter results by Y",
      "required_capabilities": [],
      "depends_on": [0]
    },
    {
      "task_type": "summarize",
      "description": "Summarize findings",
      "required_capabilities": [],
      "depends_on": [1]
    }
  ]
}"#;

        let analysis = parse_llm_response(response).unwrap();
        assert_eq!(analysis.subtask_specs.len(), 3);
        assert_eq!(analysis.subtask_specs[1].depends_on, vec![0]);
    }

    #[test]
    fn parse_response_no_json() {
        let response = "I cannot decompose this task";
        assert!(parse_llm_response(response).is_err());
    }

    #[test]
    fn parse_response_invalid_dep_index() {
        let response = r#"{
  "keywords": [],
  "entities": [],
  "subtask_specs": [
    {
      "task_type": "search",
      "description": "Search",
      "required_capabilities": [],
      "depends_on": [5]
    }
  ]
}"#;
        assert!(parse_llm_response(response).is_err());
    }

    #[test]
    fn parse_response_with_preamble_and_trailing_text() {
        let response = "Sure, here is the task decomposition:\n\n\
            {\"keywords\": [\"deploy\"], \"entities\": [], \"subtask_specs\": [\
            {\"task_type\": \"generate\", \"description\": \"Generate config\", \
            \"required_capabilities\": [], \"depends_on\": []}]}\n\n\
            Let me know if you need more details.";
        let analysis = parse_llm_response(response).unwrap();
        assert_eq!(analysis.subtask_specs.len(), 1);
        assert_eq!(analysis.subtask_specs[0].task_type, "generate");
    }

    #[test]
    fn parse_response_malformed_json() {
        let response = r#"{ "keywords": ["test"], "entities": [, "subtask_specs": [] }"#;
        assert!(parse_llm_response(response).is_err());
    }

    #[test]
    fn parse_response_empty_specs_ok() {
        let response = r#"{"keywords": ["simple"], "entities": [], "subtask_specs": []}"#;
        let analysis = parse_llm_response(response).unwrap();
        assert!(analysis.subtask_specs.is_empty());
        assert_eq!(analysis.keywords, vec!["simple"]);
    }

    #[test]
    fn parse_response_self_dependency_out_of_range() {
        let response = r#"{
  "keywords": [],
  "entities": [],
  "subtask_specs": [
    {
      "task_type": "a",
      "description": "Task A",
      "required_capabilities": [],
      "depends_on": [0]
    }
  ]
}"#;
        // depends_on: [0] when there's only 1 spec (index 0 < count 1) is technically valid
        // The spec refers to itself — this won't cause a cycle error here (that's caught
        // later by create_execution_order). parse_llm_response only validates range.
        let analysis = parse_llm_response(response).unwrap();
        assert_eq!(analysis.subtask_specs[0].depends_on, vec![0]);
    }

    #[test]
    fn parse_response_complex_diamond() {
        let response = r#"{
  "keywords": ["search", "compare"],
  "entities": [{"entity_type": "topic", "value": "rust frameworks"}],
  "subtask_specs": [
    {"task_type": "search", "description": "Search crates.io", "required_capabilities": ["web"], "depends_on": []},
    {"task_type": "search", "description": "Search GitHub", "required_capabilities": ["web"], "depends_on": []},
    {"task_type": "analyze", "description": "Compare results", "required_capabilities": [], "depends_on": [0, 1]},
    {"task_type": "summarize", "description": "Write summary", "required_capabilities": [], "depends_on": [2]}
  ]
}"#;
        let analysis = parse_llm_response(response).unwrap();
        assert_eq!(analysis.subtask_specs.len(), 4);
        assert_eq!(analysis.subtask_specs[2].depends_on, vec![0, 1]);
        assert_eq!(analysis.subtask_specs[3].depends_on, vec![2]);
        assert_eq!(analysis.entities.len(), 1);
    }

    #[test]
    fn parse_response_only_braces() {
        // Degenerate but valid JSON
        let response = "{}";
        // Missing required fields should fail serde
        assert!(parse_llm_response(response).is_err());
    }

    #[test]
    fn parse_response_nested_json_picks_outermost() {
        let response = r#"{"keywords": [], "entities": [], "subtask_specs": [
            {"task_type": "x", "description": "inner {json}", "required_capabilities": [], "depends_on": []}
        ]}"#;
        let analysis = parse_llm_response(response).unwrap();
        assert_eq!(analysis.subtask_specs.len(), 1);
        assert!(analysis.subtask_specs[0].description.contains("{json}"));
    }
}
