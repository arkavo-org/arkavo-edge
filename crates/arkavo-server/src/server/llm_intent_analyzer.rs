//! LLM-based intent analyzer. The arm is whatever the router resolved for this
//! install — a cached local model where one is provisioned, the configured cloud
//! model on a cloud-only install — so decomposition never provisions weights of
//! its own.

use arkavo_router::ModelChoice;
use arkavo_tasks::intent_analyzer::{IntentAnalysis, IntentAnalyzer};
use arkavo_tasks::task_planner::TaskPlanError;
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

/// Budget for one decomposition. It has to cover more than inference:
/// `route_fast` first waits on the router's single-permit synthesis semaphore,
/// so a decomposition queued behind another internal call spends part of this
/// budget queueing. The old 20 s ceiling was the bare generation time of a
/// sub-billion-parameter local model and left no room for that wait. A cloud
/// reasoning arm is additionally an order of magnitude slower to first token.
const LOCAL_ANALYSIS_TIMEOUT: Duration = Duration::from_secs(60);
const CLOUD_ANALYSIS_TIMEOUT: Duration = Duration::from_secs(90);

pub(super) struct LlmIntentAnalyzer {
    router: Arc<arkavo_router::Router>,
}

impl LlmIntentAnalyzer {
    pub(super) fn new(router: Arc<arkavo_router::Router>) -> Self {
        Self { router }
    }
}

fn analysis_timeout(model: &ModelChoice) -> Duration {
    if model.is_local() {
        LOCAL_ANALYSIS_TIMEOUT
    } else {
        CLOUD_ANALYSIS_TIMEOUT
    }
}

#[async_trait]
impl IntentAnalyzer for LlmIntentAnalyzer {
    async fn analyze(&self, intent: &str) -> Result<IntentAnalysis, TaskPlanError> {
        // `default_chat_model` is the router's own resolution of the execution
        // arm — a cached local model where one is provisioned, the configured
        // cloud arm otherwise — so a cloud-only install never reaches for
        // weights it would have to download. `route_fast` then dispatches that
        // same arm under the spend policy and the synthesis semaphore instead
        // of bypassing both with a raw provider.
        let model = self.router.default_chat_model();

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
            arkavo_llm::Message::system(system_prompt),
            arkavo_llm::Message::user(intent),
        ];

        let budget = analysis_timeout(&model);
        let stream =
            tokio::time::timeout(budget, self.router.route_fast("intent analysis", messages))
                .await
                .map_err(|_| {
                    TaskPlanError::LlmAnalysisFailed(format!("timeout after {}s", budget.as_secs()))
                })?
                .map_err(|e| {
                    TaskPlanError::LlmAnalysisFailed(format!("{} unavailable: {e}", model.name()))
                })?;

        let response = stream
            .complete()
            .await
            .map_err(|e| TaskPlanError::LlmAnalysisFailed(format!("LLM error: {e}")))?
            .content;

        debug!(
            model = %model.name(),
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
    use arkavo_llm::{Message, Provider, StreamResponse};
    use arkavo_router::{
        ConnectivityChecker, ModelSelector, ProviderAvailability, ProviderFactory, Router,
    };
    use arkavo_test_macros::spec;
    use futures::Stream;
    use std::sync::Mutex;

    const STUB_DECOMPOSITION: &str = r#"{"keywords":["ship"],"entities":[],"subtask_specs":[
        {"task_type":"analyze","description":"Inspect the release","required_capabilities":[],"depends_on":[]},
        {"task_type":"generate","description":"Write the notes","required_capabilities":[],"depends_on":[0]}
    ]}"#;

    /// Answers every dispatch with a fixed decomposition. Substituted for the
    /// real client so no test reaches credentials, the model cache or a network.
    struct StubProvider {
        content: String,
    }

    #[async_trait]
    impl Provider for StubProvider {
        async fn complete_with_options(
            &self,
            _messages: Vec<Message>,
            _max_tokens: Option<usize>,
        ) -> arkavo_llm::Result<String> {
            Ok(self.content.clone())
        }

        async fn stream(
            &self,
            _messages: Vec<Message>,
        ) -> arkavo_llm::Result<
            Box<dyn Stream<Item = arkavo_llm::Result<StreamResponse>> + Send + Unpin>,
        > {
            Ok(Box::new(futures::stream::empty()))
        }

        fn name(&self) -> &str {
            "stub"
        }
    }

    /// Records which arm the router asked to be built. That record is the
    /// assertion: "analysis never reached for a local provider" is observed,
    /// not inferred.
    #[derive(Clone)]
    struct RecordingFactory {
        requested: Arc<Mutex<Vec<ModelChoice>>>,
        content: String,
    }

    impl RecordingFactory {
        fn new(content: &str) -> Self {
            Self {
                requested: Arc::new(Mutex::new(Vec::new())),
                content: content.to_string(),
            }
        }

        fn handle(&self) -> Arc<dyn ProviderFactory> {
            Arc::new(self.clone())
        }

        fn requested(&self) -> Vec<ModelChoice> {
            self.requested.lock().unwrap().clone()
        }
    }

    impl ProviderFactory for RecordingFactory {
        fn build(&self, model: &ModelChoice) -> arkavo_router::Result<Box<dyn Provider>> {
            self.requested.lock().unwrap().push(model.clone());
            Ok(Box::new(StubProvider {
                content: self.content.clone(),
            }))
        }
    }

    async fn router_with(availability: ProviderAvailability, local_cached: bool) -> Router {
        Router::new()
            .await
            .expect("router")
            .with_selector(ModelSelector::with_availability(availability, local_cached))
            .await
            .with_connectivity(ConnectivityChecker::assume(true))
    }

    /// The regression: `analyze` used to call `get_provider(&LocalQwen3)` with a
    /// `LocalMinistral3B` fallback, which on a cloud-only install with nothing
    /// cached triggers a HuggingFace download. Restore that line and this test
    /// fails on the `requested` assertions.
    #[tokio::test]
    #[spec("ASTRA-004")]
    async fn cloud_only_install_never_requests_a_local_provider() {
        let availability = ProviderAvailability {
            openai: true,
            ..ProviderAvailability::default()
        };
        let factory = RecordingFactory::new(STUB_DECOMPOSITION);
        let router = router_with(availability, false)
            .await
            .with_provider_factory(factory.handle());
        router.confirm_cloud_for_session();

        let analyzer = LlmIntentAnalyzer::new(Arc::new(router));
        let analysis = analyzer
            .analyze("ship the release and write the notes")
            .await
            .expect("cloud-only analysis must succeed");
        assert_eq!(analysis.subtask_specs.len(), 2);

        let requested = factory.requested();
        assert!(!requested.is_empty(), "no provider was ever requested");
        assert!(
            requested.iter().all(|m| !m.is_local()),
            "cloud-only install requested a local arm: {requested:?}"
        );
        assert!(
            !requested.contains(&ModelChoice::LocalQwen3),
            "{requested:?}"
        );
        assert!(
            !requested.contains(&ModelChoice::LocalMinistral3B),
            "{requested:?}"
        );
    }

    /// Mirror: an install that has weights and no cloud credentials keeps
    /// running locally, so the fix did not simply push everyone to the cloud.
    #[tokio::test]
    #[spec("ASTRA-004")]
    async fn cached_local_install_still_requests_a_local_provider() {
        let factory = RecordingFactory::new(STUB_DECOMPOSITION);
        let router = router_with(ProviderAvailability::default(), true)
            .await
            .with_provider_factory(factory.handle());

        let analyzer = LlmIntentAnalyzer::new(Arc::new(router));
        analyzer
            .analyze("ship the release and write the notes")
            .await
            .expect("local analysis must succeed");

        let requested = factory.requested();
        assert!(!requested.is_empty(), "no provider was ever requested");
        assert!(
            requested.iter().all(ModelChoice::is_local),
            "install with cached weights and no cloud went off-device: {requested:?}"
        );
    }

    #[test]
    #[spec("ASTRA-004")]
    fn cloud_arm_gets_a_longer_budget_than_the_local_one() {
        assert_eq!(
            analysis_timeout(&ModelChoice::LocalQwen3),
            LOCAL_ANALYSIS_TIMEOUT
        );
        assert_eq!(
            analysis_timeout(&ModelChoice::Gpt6Astra),
            CLOUD_ANALYSIS_TIMEOUT
        );
        assert!(CLOUD_ANALYSIS_TIMEOUT > LOCAL_ANALYSIS_TIMEOUT);
    }

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
