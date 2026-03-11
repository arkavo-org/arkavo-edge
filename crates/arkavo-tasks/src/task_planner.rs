//! Task planner that decomposes complex tasks into subtasks and creates execution plans.

use crate::agent_registry::AgentRegistry;
use crate::intent_analyzer::{IntentAnalysis, IntentAnalyzer, RuleBasedAnalyzer, SubTaskSpec};
use crate::types::{DeviceCapabilities, TaskOffer};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Task planner that decomposes complex tasks into subtasks and creates execution plans.
pub struct TaskPlanner {
    agent_registry: Arc<AgentRegistry>,
    intent_analyzer: Box<dyn IntentAnalyzer>,
}

/// A planned task with subtasks and execution order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPlan {
    pub plan_id: Uuid,
    pub task_id: Uuid,
    pub intent: String,
    pub device_caps: Option<DeviceCapabilities>,
    pub subtasks: Vec<SubTask>,
    /// Execution order (array of arrays for parallel stages)
    pub execution_order: Vec<Vec<Uuid>>,
    pub dependencies: HashMap<Uuid, Vec<Uuid>>,
}

/// A subtask within a task plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTask {
    pub id: Uuid,
    pub task_type: String,
    pub description: String,
    pub required_capabilities: Vec<String>,
    pub input_data: serde_json::Value,
    pub assigned_agent: Option<String>,
    pub status: SubTaskStatus,
    pub output: Option<serde_json::Value>,
}

/// Status of a subtask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubTaskStatus {
    Pending,
    Assigned,
    Running,
    Completed,
    Failed,
}

/// Error types for task planning.
#[derive(Debug, thiserror::Error)]
pub enum TaskPlanError {
    #[error("No agents available with required capability: {0}")]
    NoAgentsAvailable(String),

    #[error("Circular dependency detected in task plan")]
    CircularDependency,

    #[error("Invalid intent: {0}")]
    InvalidIntent(String),

    #[error("Agent registry error: {0}")]
    RegistryError(String),

    #[error("LLM intent analysis failed: {0}")]
    LlmAnalysisFailed(String),
}

impl TaskPlanner {
    /// Create a new task planner with the default rule-based analyzer.
    pub fn new(agent_registry: Arc<AgentRegistry>) -> Self {
        Self {
            agent_registry,
            intent_analyzer: Box::new(RuleBasedAnalyzer),
        }
    }

    /// Create a task planner with a custom intent analyzer.
    pub fn with_analyzer(
        agent_registry: Arc<AgentRegistry>,
        intent_analyzer: Box<dyn IntentAnalyzer>,
    ) -> Self {
        Self {
            agent_registry,
            intent_analyzer,
        }
    }

    /// Plan a task from a TaskOffer (legacy entry point with agent assignment).
    pub async fn plan_task(&self, task_offer: TaskOffer) -> Result<TaskPlan, TaskPlanError> {
        info!(intent = %task_offer.intent, "Planning task");

        let analysis = self.intent_analyzer.analyze(&task_offer.intent).await?;

        debug!(
            keywords = ?analysis.keywords,
            entities = ?analysis.entities,
            specs = analysis.subtask_specs.len(),
            "Intent analysis complete"
        );

        let (mut subtasks, dependencies) = self.build_subtasks(&task_offer, &analysis)?;
        let execution_order = self.create_execution_order(&subtasks, &dependencies)?;
        self.assign_agents_to_subtasks(&mut subtasks).await?;

        let plan = TaskPlan {
            plan_id: Uuid::new_v4(),
            task_id: Uuid::parse_str(&task_offer.intent_id).unwrap_or_else(|_| Uuid::new_v4()),
            intent: task_offer.intent,
            device_caps: Some(task_offer.device_caps),
            subtasks,
            execution_order,
            dependencies,
        };

        info!(
            plan.id = %plan.plan_id,
            subtasks = plan.subtasks.len(),
            stages = plan.execution_order.len(),
            "Task plan created"
        );

        Ok(plan)
    }

    /// Plan subtasks from an intent string without agent assignment.
    /// Returns `(subtasks, execution_order, dependencies)`.
    /// Used by the conductor which executes subtasks via its own tool loop.
    pub async fn plan_subtasks(
        &self,
        intent: &str,
    ) -> Result<(Vec<SubTask>, Vec<Vec<Uuid>>, HashMap<Uuid, Vec<Uuid>>), TaskPlanError> {
        info!(intent = %intent, "Planning subtasks");

        let analysis = self.intent_analyzer.analyze(intent).await?;

        let (subtasks, dependencies) = if !analysis.subtask_specs.is_empty() {
            create_subtasks_from_specs(&analysis.subtask_specs)
        } else {
            let subtasks = self.create_subtasks_from_analysis(&analysis)?;
            let dependencies = self.analyze_dependencies(&subtasks);
            (subtasks, dependencies)
        };

        let execution_order = self.create_execution_order(&subtasks, &dependencies)?;

        info!(
            subtasks = subtasks.len(),
            stages = execution_order.len(),
            "Subtask plan created"
        );

        Ok((subtasks, execution_order, dependencies))
    }

    /// Plan from a plain intent string, building a TaskOffer internally.
    pub async fn plan_from_intent(&self, intent: &str) -> Result<TaskPlan, TaskPlanError> {
        let task_offer = TaskOffer {
            task_id: Uuid::new_v4(),
            agent_id: "conductor".to_string(),
            price: None,
            time_estimate_seconds: None,
            intent_id: Uuid::new_v4().to_string(),
            intent: intent.to_string(),
            device_caps: DeviceCapabilities::default(),
        };
        self.plan_task(task_offer).await
    }

    /// Build subtasks from intent analysis, choosing spec-based or legacy path.
    #[allow(clippy::type_complexity)]
    fn build_subtasks(
        &self,
        task_offer: &TaskOffer,
        analysis: &IntentAnalysis,
    ) -> Result<(Vec<SubTask>, HashMap<Uuid, Vec<Uuid>>), TaskPlanError> {
        if !analysis.subtask_specs.is_empty() {
            Ok(create_subtasks_from_specs(&analysis.subtask_specs))
        } else {
            let subtasks = self.create_subtasks_from_analysis(analysis)?;
            let dependencies = self.analyze_dependencies(&subtasks);
            let _ = task_offer; // used for future context
            Ok((subtasks, dependencies))
        }
    }

    /// Create subtasks from rule-based intent analysis (legacy path).
    fn create_subtasks_from_analysis(
        &self,
        analysis: &IntentAnalysis,
    ) -> Result<Vec<SubTask>, TaskPlanError> {
        let mut subtasks = Vec::new();

        if analysis.keywords.contains(&"location".to_string()) {
            subtasks.push(SubTask {
                id: Uuid::new_v4(),
                task_type: "sensor_request".to_string(),
                description: "Get current location".to_string(),
                required_capabilities: vec!["location".to_string()],
                input_data: serde_json::json!({
                    "sensor": "location",
                    "scope": "standard"
                }),
                assigned_agent: None,
                status: SubTaskStatus::Pending,
                output: None,
            });
        }

        if analysis.keywords.contains(&"search".to_string()) {
            let search_query = build_search_query(analysis);
            subtasks.push(SubTask {
                id: Uuid::new_v4(),
                task_type: "web_search".to_string(),
                description: format!("Search for: {search_query}"),
                required_capabilities: vec!["web_search".to_string()],
                input_data: serde_json::json!({
                    "query": search_query,
                    "location_ref": "$location"
                }),
                assigned_agent: None,
                status: SubTaskStatus::Pending,
                output: None,
            });
        }

        for entity in &analysis.entities {
            if entity.entity_type == "amenity" || entity.entity_type == "filter_criterion" {
                subtasks.push(SubTask {
                    id: Uuid::new_v4(),
                    task_type: "filter".to_string(),
                    description: format!("Filter by {}: {}", entity.entity_type, entity.value),
                    required_capabilities: vec!["data_processing".to_string()],
                    input_data: serde_json::json!({
                        "filter_type": entity.entity_type,
                        "filter_value": entity.value,
                        "input_ref": "$search_results"
                    }),
                    assigned_agent: None,
                    status: SubTaskStatus::Pending,
                    output: None,
                });
            }
        }

        if analysis.keywords.contains(&"summarize".to_string()) || !subtasks.is_empty() {
            subtasks.push(SubTask {
                id: Uuid::new_v4(),
                task_type: "summarize".to_string(),
                description: "Summarize results".to_string(),
                required_capabilities: vec!["foundation_models".to_string()],
                input_data: serde_json::json!({
                    "text_ref": "$filtered_results",
                    "length": "medium"
                }),
                assigned_agent: None,
                status: SubTaskStatus::Pending,
                output: None,
            });
        }

        if subtasks.is_empty() {
            return Err(TaskPlanError::InvalidIntent(
                "Could not create any subtasks from intent".to_string(),
            ));
        }

        Ok(subtasks)
    }

    /// Analyze dependencies between subtasks using hardcoded $-token references (legacy path).
    fn analyze_dependencies(&self, subtasks: &[SubTask]) -> HashMap<Uuid, Vec<Uuid>> {
        let mut dependencies = HashMap::new();

        for (i, subtask) in subtasks.iter().enumerate() {
            let mut deps = Vec::new();
            let input_str = subtask.input_data.to_string();

            if input_str.contains("$location")
                && let Some(location_task) =
                    subtasks.iter().find(|t| t.task_type == "sensor_request")
            {
                deps.push(location_task.id);
            }

            if input_str.contains("$search_results")
                && let Some(search_task) = subtasks.iter().find(|t| t.task_type == "web_search")
            {
                deps.push(search_task.id);
            }

            if input_str.contains("$filtered_results") {
                for prev_subtask in subtasks.iter().take(i) {
                    if prev_subtask.task_type == "filter" {
                        deps.push(prev_subtask.id);
                    }
                }
            }

            if !deps.is_empty() {
                dependencies.insert(subtask.id, deps);
            }
        }

        dependencies
    }

    /// Create execution order using topological sort.
    fn create_execution_order(
        &self,
        subtasks: &[SubTask],
        dependencies: &HashMap<Uuid, Vec<Uuid>>,
    ) -> Result<Vec<Vec<Uuid>>, TaskPlanError> {
        let mut stages: Vec<Vec<Uuid>> = Vec::new();
        let mut completed: Vec<Uuid> = Vec::new();
        let mut remaining: Vec<Uuid> = subtasks.iter().map(|t| t.id).collect();

        while !remaining.is_empty() {
            let mut current_stage = Vec::new();

            for task_id in &remaining {
                let deps = dependencies.get(task_id);
                let all_deps_met = match deps {
                    None => true,
                    Some(deps) => deps.iter().all(|dep_id| completed.contains(dep_id)),
                };

                if all_deps_met {
                    current_stage.push(*task_id);
                }
            }

            if current_stage.is_empty() {
                return Err(TaskPlanError::CircularDependency);
            }

            remaining.retain(|id| !current_stage.contains(id));
            completed.extend(current_stage.iter());
            stages.push(current_stage);
        }

        Ok(stages)
    }

    /// Assign agents to subtasks based on required capabilities.
    async fn assign_agents_to_subtasks(
        &self,
        subtasks: &mut [SubTask],
    ) -> Result<(), TaskPlanError> {
        for subtask in subtasks.iter_mut() {
            let agent_id = if subtask.required_capabilities.len() == 1 {
                self.agent_registry
                    .find_best_agent(&subtask.required_capabilities[0])
                    .await
            } else {
                let agents = self
                    .agent_registry
                    .find_agents_with_all_capabilities(&subtask.required_capabilities)
                    .await;
                agents.first().cloned()
            };

            match agent_id {
                Some(id) => {
                    subtask.assigned_agent = Some(id.clone());
                    subtask.status = SubTaskStatus::Assigned;
                    debug!(
                        subtask.id = %subtask.id,
                        agent.id = %id,
                        "Assigned agent to subtask"
                    );
                }
                None => {
                    warn!(
                        subtask.id = %subtask.id,
                        capabilities = ?subtask.required_capabilities,
                        "No agent available for subtask"
                    );
                    return Err(TaskPlanError::NoAgentsAvailable(
                        subtask.required_capabilities.join(", "),
                    ));
                }
            }
        }

        Ok(())
    }
}

/// Build search query from intent analysis entities.
fn build_search_query(analysis: &IntentAnalysis) -> String {
    let mut query_parts = Vec::new();
    for entity in &analysis.entities {
        if entity.entity_type == "place_type" {
            query_parts.push(entity.value.clone());
        }
    }
    if query_parts.is_empty() {
        "search query".to_string()
    } else {
        query_parts.join(" ")
    }
}

/// Create subtasks from LLM-generated SubTaskSpecs.
/// Returns subtasks and pre-built dependency map (bypasses `analyze_dependencies`).
pub fn create_subtasks_from_specs(
    specs: &[SubTaskSpec],
) -> (Vec<SubTask>, HashMap<Uuid, Vec<Uuid>>) {
    let ids: Vec<Uuid> = specs.iter().map(|_| Uuid::new_v4()).collect();
    let mut subtasks = Vec::with_capacity(specs.len());
    let mut dependencies = HashMap::new();

    for (i, spec) in specs.iter().enumerate() {
        subtasks.push(SubTask {
            id: ids[i],
            task_type: spec.task_type.clone(),
            description: spec.description.clone(),
            required_capabilities: spec.required_capabilities.clone(),
            input_data: serde_json::json!({}),
            assigned_agent: None,
            status: SubTaskStatus::Pending,
            output: None,
        });

        let deps: Vec<Uuid> = spec
            .depends_on
            .iter()
            .filter_map(|&idx| ids.get(idx).copied())
            .collect();

        if !deps.is_empty() {
            dependencies.insert(ids[i], deps);
        }
    }

    (subtasks, dependencies)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_intent_analysis_via_rule_based() {
        let planner = create_test_planner().await;
        let (subtasks, _, _) = planner
            .plan_subtasks("Find nearby coffee shops with outdoor seating")
            .await
            .unwrap();

        assert!(!subtasks.is_empty());
        assert!(subtasks.iter().any(|s| s.task_type == "sensor_request"));
        assert!(subtasks.iter().any(|s| s.task_type == "web_search"));
    }

    #[tokio::test]
    async fn test_task_planning() {
        let planner = create_test_planner().await;

        let task_offer = TaskOffer {
            task_id: Uuid::new_v4(),
            agent_id: "test-agent".to_string(),
            price: Some(1.0),
            time_estimate_seconds: Some(60),
            intent_id: Uuid::new_v4().to_string(),
            intent: "Find nearby coffee shops".to_string(),
            device_caps: DeviceCapabilities {
                can_execute_code: false,
                can_access_files: true,
                can_make_http_requests: true,
            },
        };

        let plan = planner.plan_task(task_offer).await.unwrap();

        assert!(!plan.subtasks.is_empty());
        assert!(!plan.execution_order.is_empty());
    }

    #[test]
    fn test_subtask_specs_to_subtasks() {
        let specs = vec![
            SubTaskSpec {
                task_type: "search".to_string(),
                description: "Search for restaurants".to_string(),
                required_capabilities: vec!["web_search".to_string()],
                depends_on: vec![],
            },
            SubTaskSpec {
                task_type: "filter".to_string(),
                description: "Filter by rating".to_string(),
                required_capabilities: vec!["data_processing".to_string()],
                depends_on: vec![0],
            },
            SubTaskSpec {
                task_type: "summarize".to_string(),
                description: "Summarize top results".to_string(),
                required_capabilities: vec!["foundation_models".to_string()],
                depends_on: vec![1],
            },
        ];

        let (subtasks, deps) = create_subtasks_from_specs(&specs);

        assert_eq!(subtasks.len(), 3);
        assert_eq!(subtasks[0].task_type, "search");
        assert_eq!(subtasks[1].task_type, "filter");
        assert_eq!(subtasks[2].task_type, "summarize");

        // filter depends on search
        assert!(deps.contains_key(&subtasks[1].id));
        assert_eq!(deps[&subtasks[1].id], vec![subtasks[0].id]);

        // summarize depends on filter
        assert!(deps.contains_key(&subtasks[2].id));
        assert_eq!(deps[&subtasks[2].id], vec![subtasks[1].id]);

        // search has no deps
        assert!(!deps.contains_key(&subtasks[0].id));
    }

    #[test]
    fn test_subtask_specs_out_of_range_dep_ignored() {
        let specs = vec![SubTaskSpec {
            task_type: "search".to_string(),
            description: "Search".to_string(),
            required_capabilities: vec![],
            depends_on: vec![5], // out of range
        }];

        let (subtasks, deps) = create_subtasks_from_specs(&specs);
        assert_eq!(subtasks.len(), 1);
        assert!(!deps.contains_key(&subtasks[0].id));
    }

    #[tokio::test]
    async fn test_plan_subtasks_no_agents_needed() {
        let registry = Arc::new(AgentRegistry::new());
        let planner = TaskPlanner::new(registry);

        let result = planner.plan_subtasks("Find nearby coffee shops").await;

        // plan_subtasks doesn't need agents in registry — it skips assignment
        assert!(result.is_ok());
        let (subtasks, order, _) = result.unwrap();
        assert!(!subtasks.is_empty());
        assert!(!order.is_empty());
    }

    #[tokio::test]
    async fn test_plan_subtasks_with_custom_analyzer() {
        use crate::intent_analyzer::{IntentAnalysis, SubTaskSpec};
        use async_trait::async_trait;

        struct MockAnalyzer;

        #[async_trait]
        impl IntentAnalyzer for MockAnalyzer {
            async fn analyze(&self, _intent: &str) -> Result<IntentAnalysis, TaskPlanError> {
                Ok(IntentAnalysis {
                    keywords: vec!["search".to_string()],
                    entities: vec![],
                    subtask_specs: vec![
                        SubTaskSpec {
                            task_type: "search".to_string(),
                            description: "Search the web".to_string(),
                            required_capabilities: vec![],
                            depends_on: vec![],
                        },
                        SubTaskSpec {
                            task_type: "analyze".to_string(),
                            description: "Analyze search results".to_string(),
                            required_capabilities: vec![],
                            depends_on: vec![0],
                        },
                        SubTaskSpec {
                            task_type: "summarize".to_string(),
                            description: "Summarize findings".to_string(),
                            required_capabilities: vec![],
                            depends_on: vec![1],
                        },
                    ],
                })
            }
        }

        let registry = Arc::new(AgentRegistry::new());
        let planner = TaskPlanner::with_analyzer(registry, Box::new(MockAnalyzer));

        let (subtasks, order, deps) = planner.plan_subtasks("anything").await.unwrap();

        assert_eq!(subtasks.len(), 3);
        assert_eq!(order.len(), 3); // linear chain → 3 stages

        // Verify stage order: search → analyze → summarize
        assert_eq!(order[0].len(), 1);
        assert_eq!(order[1].len(), 1);
        assert_eq!(order[2].len(), 1);

        // Verify dependency chain
        let search_id = subtasks[0].id;
        let analyze_id = subtasks[1].id;
        let summarize_id = subtasks[2].id;

        assert!(!deps.contains_key(&search_id));
        assert_eq!(deps[&analyze_id], vec![search_id]);
        assert_eq!(deps[&summarize_id], vec![analyze_id]);
    }

    #[tokio::test]
    async fn test_plan_subtasks_parallel_stages() {
        use crate::intent_analyzer::{IntentAnalysis, SubTaskSpec};
        use async_trait::async_trait;

        struct ParallelAnalyzer;

        #[async_trait]
        impl IntentAnalyzer for ParallelAnalyzer {
            async fn analyze(&self, _intent: &str) -> Result<IntentAnalysis, TaskPlanError> {
                Ok(IntentAnalysis {
                    keywords: vec![],
                    entities: vec![],
                    subtask_specs: vec![
                        SubTaskSpec {
                            task_type: "search_a".to_string(),
                            description: "Search source A".to_string(),
                            required_capabilities: vec![],
                            depends_on: vec![],
                        },
                        SubTaskSpec {
                            task_type: "search_b".to_string(),
                            description: "Search source B".to_string(),
                            required_capabilities: vec![],
                            depends_on: vec![],
                        },
                        SubTaskSpec {
                            task_type: "merge".to_string(),
                            description: "Merge results from A and B".to_string(),
                            required_capabilities: vec![],
                            depends_on: vec![0, 1],
                        },
                    ],
                })
            }
        }

        let registry = Arc::new(AgentRegistry::new());
        let planner = TaskPlanner::with_analyzer(registry, Box::new(ParallelAnalyzer));

        let (subtasks, order, deps) = planner.plan_subtasks("parallel search").await.unwrap();

        assert_eq!(subtasks.len(), 3);
        // Stage 1: search_a and search_b in parallel
        // Stage 2: merge
        assert_eq!(order.len(), 2);
        assert_eq!(order[0].len(), 2); // parallel
        assert_eq!(order[1].len(), 1); // merge

        // Merge depends on both searches
        let merge_id = subtasks[2].id;
        assert_eq!(deps[&merge_id].len(), 2);
    }

    #[tokio::test]
    async fn test_plan_subtasks_circular_dependency() {
        use crate::intent_analyzer::{IntentAnalysis, SubTaskSpec};
        use async_trait::async_trait;

        struct CircularAnalyzer;

        #[async_trait]
        impl IntentAnalyzer for CircularAnalyzer {
            async fn analyze(&self, _intent: &str) -> Result<IntentAnalysis, TaskPlanError> {
                Ok(IntentAnalysis {
                    keywords: vec![],
                    entities: vec![],
                    subtask_specs: vec![
                        SubTaskSpec {
                            task_type: "a".to_string(),
                            description: "Task A".to_string(),
                            required_capabilities: vec![],
                            depends_on: vec![1], // A depends on B
                        },
                        SubTaskSpec {
                            task_type: "b".to_string(),
                            description: "Task B".to_string(),
                            required_capabilities: vec![],
                            depends_on: vec![0], // B depends on A → circular
                        },
                    ],
                })
            }
        }

        let registry = Arc::new(AgentRegistry::new());
        let planner = TaskPlanner::with_analyzer(registry, Box::new(CircularAnalyzer));

        let result = planner.plan_subtasks("loop").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            TaskPlanError::CircularDependency => {} // expected
            other => panic!("Expected CircularDependency, got: {other}"),
        }
    }

    #[tokio::test]
    async fn test_plan_subtasks_empty_intent_rule_based() {
        let registry = Arc::new(AgentRegistry::new());
        let planner = TaskPlanner::new(registry);

        let result = planner.plan_subtasks("hello world").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            TaskPlanError::InvalidIntent(_) => {} // expected
            other => panic!("Expected InvalidIntent, got: {other}"),
        }
    }

    #[tokio::test]
    async fn test_plan_subtasks_analyzer_error_propagated() {
        use async_trait::async_trait;

        struct FailingAnalyzer;

        #[async_trait]
        impl IntentAnalyzer for FailingAnalyzer {
            async fn analyze(
                &self,
                _intent: &str,
            ) -> Result<crate::intent_analyzer::IntentAnalysis, TaskPlanError> {
                Err(TaskPlanError::LlmAnalysisFailed("test error".into()))
            }
        }

        let registry = Arc::new(AgentRegistry::new());
        let planner = TaskPlanner::with_analyzer(registry, Box::new(FailingAnalyzer));

        let result = planner.plan_subtasks("anything").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            TaskPlanError::LlmAnalysisFailed(msg) => assert_eq!(msg, "test error"),
            other => panic!("Expected LlmAnalysisFailed, got: {other}"),
        }
    }

    #[tokio::test]
    async fn test_plan_from_intent_needs_agents() {
        let registry = Arc::new(AgentRegistry::new());
        let planner = TaskPlanner::new(registry);

        // plan_from_intent calls plan_task which needs agent assignment
        let result = planner.plan_from_intent("Find nearby coffee shops").await;
        // Fails because no agents registered
        assert!(result.is_err());
        match result.unwrap_err() {
            TaskPlanError::NoAgentsAvailable(_) => {} // expected
            other => panic!("Expected NoAgentsAvailable, got: {other}"),
        }
    }

    #[test]
    fn test_subtask_specs_preserves_descriptions() {
        let specs = vec![SubTaskSpec {
            task_type: "analyze".to_string(),
            description: "Analyze the codebase for security issues".to_string(),
            required_capabilities: vec!["security".to_string()],
            depends_on: vec![],
        }];

        let (subtasks, _) = create_subtasks_from_specs(&specs);
        assert_eq!(
            subtasks[0].description,
            "Analyze the codebase for security issues"
        );
        assert_eq!(subtasks[0].required_capabilities, vec!["security"]);
        assert_eq!(subtasks[0].status, SubTaskStatus::Pending);
        assert!(subtasks[0].assigned_agent.is_none());
        assert!(subtasks[0].output.is_none());
    }

    #[test]
    fn test_subtask_specs_diamond_dependency() {
        //     A
        //    / \
        //   B   C
        //    \ /
        //     D
        let specs = vec![
            SubTaskSpec {
                task_type: "a".to_string(),
                description: "Root".to_string(),
                required_capabilities: vec![],
                depends_on: vec![],
            },
            SubTaskSpec {
                task_type: "b".to_string(),
                description: "Left".to_string(),
                required_capabilities: vec![],
                depends_on: vec![0],
            },
            SubTaskSpec {
                task_type: "c".to_string(),
                description: "Right".to_string(),
                required_capabilities: vec![],
                depends_on: vec![0],
            },
            SubTaskSpec {
                task_type: "d".to_string(),
                description: "Join".to_string(),
                required_capabilities: vec![],
                depends_on: vec![1, 2],
            },
        ];

        let (subtasks, deps) = create_subtasks_from_specs(&specs);
        assert_eq!(subtasks.len(), 4);

        let a_id = subtasks[0].id;
        let b_id = subtasks[1].id;
        let c_id = subtasks[2].id;
        let d_id = subtasks[3].id;

        assert!(!deps.contains_key(&a_id));
        assert_eq!(deps[&b_id], vec![a_id]);
        assert_eq!(deps[&c_id], vec![a_id]);
        assert_eq!(deps[&d_id].len(), 2);
        assert!(deps[&d_id].contains(&b_id));
        assert!(deps[&d_id].contains(&c_id));
    }

    #[test]
    fn test_subtask_specs_empty_produces_empty() {
        let (subtasks, deps) = create_subtasks_from_specs(&[]);
        assert!(subtasks.is_empty());
        assert!(deps.is_empty());
    }

    #[test]
    fn test_legacy_dependency_analysis_chain() {
        // The legacy path: sensor → web_search → filter → summarize
        let registry = Arc::new(AgentRegistry::new());
        let planner = TaskPlanner::new(registry);

        let analysis = crate::intent_analyzer::IntentAnalysis {
            keywords: vec![
                "location".to_string(),
                "search".to_string(),
                "filter".to_string(),
            ],
            entities: vec![
                crate::intent_analyzer::Entity {
                    entity_type: "place_type".to_string(),
                    value: "coffee shop".to_string(),
                },
                crate::intent_analyzer::Entity {
                    entity_type: "amenity".to_string(),
                    value: "outdoor seating".to_string(),
                },
            ],
            subtask_specs: vec![],
        };

        let subtasks = planner.create_subtasks_from_analysis(&analysis).unwrap();
        let deps = planner.analyze_dependencies(&subtasks);

        // Should have: sensor_request, web_search, filter, summarize
        assert_eq!(subtasks.len(), 4);
        assert_eq!(subtasks[0].task_type, "sensor_request");
        assert_eq!(subtasks[1].task_type, "web_search");
        assert_eq!(subtasks[2].task_type, "filter");
        assert_eq!(subtasks[3].task_type, "summarize");

        // web_search depends on sensor_request (via $location)
        assert!(deps[&subtasks[1].id].contains(&subtasks[0].id));

        // filter depends on web_search (via $search_results)
        assert!(deps[&subtasks[2].id].contains(&subtasks[1].id));

        // summarize depends on filter (via $filtered_results)
        assert!(deps[&subtasks[3].id].contains(&subtasks[2].id));
    }

    async fn create_test_planner() -> TaskPlanner {
        let registry = Arc::new(AgentRegistry::new());

        registry
            .register_agent(
                "local_ai".to_string(),
                "LocalAI".to_string(),
                "On-device AI".to_string(),
                vec!["location".to_string(), "foundation_models".to_string()],
                None,
                HashMap::new(),
                None,
            )
            .await
            .unwrap();

        registry
            .register_agent(
                "search_agent".to_string(),
                "Search Agent".to_string(),
                "Web search".to_string(),
                vec!["web_search".to_string()],
                None,
                HashMap::new(),
                None,
            )
            .await
            .unwrap();

        TaskPlanner::new(registry)
    }
}
