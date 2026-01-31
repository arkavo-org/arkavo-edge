//! Task planner that decomposes complex tasks into subtasks and creates execution plans.

use crate::agent_registry::AgentRegistry;
use crate::types::{DeviceCapabilities, TaskOffer};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Task planner that decomposes complex tasks into subtasks and creates execution plans.
pub struct TaskPlanner {
    agent_registry: Arc<AgentRegistry>,
}

/// A planned task with subtasks and execution order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPlan {
    /// Unique identifier for this plan
    pub plan_id: Uuid,

    /// Original task ID from TaskOffer
    pub task_id: Uuid,

    /// Original intent
    pub intent: String,

    /// Device capabilities provided by requester
    pub device_caps: Option<DeviceCapabilities>,

    /// Subtasks to execute
    pub subtasks: Vec<SubTask>,

    /// Execution order (array of arrays for parallel stages)
    /// Example: [[subtask1_id, subtask2_id], [subtask3_id]]
    /// Stage 1: subtask1 and subtask2 run in parallel
    /// Stage 2: subtask3 runs after stage 1 completes
    pub execution_order: Vec<Vec<Uuid>>,

    /// Dependencies between subtasks (subtask_id -> [depends_on_subtask_ids])
    pub dependencies: HashMap<Uuid, Vec<Uuid>>,
}

/// A subtask within a task plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTask {
    /// Unique identifier for this subtask
    pub id: Uuid,

    /// Type of task (e.g., "sensor_request", "web_search", "filter", "summarize")
    pub task_type: String,

    /// Required capabilities for this subtask
    pub required_capabilities: Vec<String>,

    /// Input data for this subtask (can reference outputs from dependencies)
    pub input_data: serde_json::Value,

    /// Agent assigned to execute this subtask
    pub assigned_agent: Option<String>,

    /// Execution status
    pub status: SubTaskStatus,

    /// Output from executing this subtask
    pub output: Option<serde_json::Value>,
}

/// Status of a subtask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubTaskStatus {
    /// Pending assignment
    Pending,
    /// Assigned to an agent
    Assigned,
    /// Currently running
    Running,
    /// Completed successfully
    Completed,
    /// Failed
    Failed,
}

/// Error types for task planning.
#[derive(Debug, thiserror::Error)]
pub enum TaskPlanError {
    /// No agents available with required capability
    #[error("No agents available with required capability: {0}")]
    NoAgentsAvailable(String),

    /// Circular dependency detected in task plan
    #[error("Circular dependency detected in task plan")]
    CircularDependency,

    /// Invalid intent for task planning
    #[error("Invalid intent: {0}")]
    InvalidIntent(String),

    /// Agent registry error
    #[error("Agent registry error: {0}")]
    RegistryError(String),
}

impl TaskPlanner {
    /// Create a new task planner.
    pub fn new(agent_registry: Arc<AgentRegistry>) -> Self {
        Self { agent_registry }
    }

    /// Plan a task from a TaskOffer.
    pub async fn plan_task(&self, task_offer: TaskOffer) -> Result<TaskPlan, TaskPlanError> {
        info!(
            intent = %task_offer.intent,
            "Planning task"
        );

        // Parse intent to extract keywords and entities
        let intent_analysis = self.analyze_intent(&task_offer.intent);

        debug!(
            keywords = ?intent_analysis.keywords,
            entities = ?intent_analysis.entities,
            "Intent analysis complete"
        );

        // Create subtasks based on intent
        let mut subtasks = self.create_subtasks(&task_offer, &intent_analysis)?;

        // Analyze dependencies between subtasks
        let dependencies = self.analyze_dependencies(&subtasks);

        // Create execution order (topological sort)
        let execution_order = self.create_execution_order(&subtasks, &dependencies)?;

        // Assign agents to subtasks
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

    /// Analyze intent to extract keywords and entities.
    fn analyze_intent(&self, intent: &str) -> IntentAnalysis {
        let intent_lower = intent.to_lowercase();

        // Extract keywords (simplified - in production use NLP)
        let mut keywords = Vec::new();
        let mut entities = Vec::new();

        // Location-related keywords
        if intent_lower.contains("nearby") || intent_lower.contains("near me") {
            keywords.push("location".to_string());
            keywords.push("search".to_string());
        }

        // Search keywords
        if intent_lower.contains("find") || intent_lower.contains("search") {
            keywords.push("search".to_string());
        }

        // Filter keywords
        if intent_lower.contains("with") || intent_lower.contains("that have") {
            keywords.push("filter".to_string());
        }

        // Summarize keywords
        if intent_lower.contains("summarize") || intent_lower.contains("summary") {
            keywords.push("summarize".to_string());
        }

        // Extract entities (simplified)
        if intent_lower.contains("coffee") {
            entities.push(Entity {
                entity_type: "place_type".to_string(),
                value: "coffee shop".to_string(),
            });
        }

        if intent_lower.contains("outdoor seating") {
            entities.push(Entity {
                entity_type: "amenity".to_string(),
                value: "outdoor seating".to_string(),
            });
        }

        if intent_lower.contains("reviews") || intent_lower.contains("rating") {
            entities.push(Entity {
                entity_type: "filter_criterion".to_string(),
                value: "reviews".to_string(),
            });
        }

        IntentAnalysis { keywords, entities }
    }

    /// Create subtasks based on intent analysis.
    fn create_subtasks(
        &self,
        _task_offer: &TaskOffer,
        intent_analysis: &IntentAnalysis,
    ) -> Result<Vec<SubTask>, TaskPlanError> {
        let mut subtasks = Vec::new();

        // If location is needed, create sensor request subtask
        if intent_analysis.keywords.contains(&"location".to_string()) {
            subtasks.push(SubTask {
                id: Uuid::new_v4(),
                task_type: "sensor_request".to_string(),
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

        // If search is needed, create web search subtask
        if intent_analysis.keywords.contains(&"search".to_string()) {
            let search_query = self.build_search_query(intent_analysis);
            subtasks.push(SubTask {
                id: Uuid::new_v4(),
                task_type: "web_search".to_string(),
                required_capabilities: vec!["web_search".to_string()],
                input_data: serde_json::json!({
                    "query": search_query,
                    "location_ref": "$location" // Reference to location subtask output
                }),
                assigned_agent: None,
                status: SubTaskStatus::Pending,
                output: None,
            });
        }

        // If filtering is needed, create filter subtasks
        for entity in &intent_analysis.entities {
            if entity.entity_type == "amenity" || entity.entity_type == "filter_criterion" {
                subtasks.push(SubTask {
                    id: Uuid::new_v4(),
                    task_type: "filter".to_string(),
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

        // Always add summarization as final step
        if intent_analysis.keywords.contains(&"summarize".to_string()) || !subtasks.is_empty() {
            subtasks.push(SubTask {
                id: Uuid::new_v4(),
                task_type: "summarize".to_string(),
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

    /// Build search query from intent analysis.
    fn build_search_query(&self, intent_analysis: &IntentAnalysis) -> String {
        let mut query_parts = Vec::new();

        for entity in &intent_analysis.entities {
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

    /// Analyze dependencies between subtasks.
    fn analyze_dependencies(&self, subtasks: &[SubTask]) -> HashMap<Uuid, Vec<Uuid>> {
        let mut dependencies = HashMap::new();

        for (i, subtask) in subtasks.iter().enumerate() {
            let mut deps = Vec::new();

            // Check if this subtask references outputs from previous subtasks
            let input_str = subtask.input_data.to_string();

            // Look for references like "$location", "$search_results", etc.
            if input_str.contains("$location") {
                // Find the location subtask
                if let Some(location_task) =
                    subtasks.iter().find(|t| t.task_type == "sensor_request")
                {
                    deps.push(location_task.id);
                }
            }

            if input_str.contains("$search_results") {
                // Find the search subtask
                if let Some(search_task) = subtasks.iter().find(|t| t.task_type == "web_search") {
                    deps.push(search_task.id);
                }
            }

            if input_str.contains("$filtered_results") {
                // Depends on all filter tasks before this one
                for subtask in subtasks.iter().take(i) {
                    if subtask.task_type == "filter" {
                        deps.push(subtask.id);
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

            // Find all tasks that have all dependencies met
            for task_id in &remaining {
                let deps = dependencies.get(task_id);
                let all_deps_met = match deps {
                    None => true, // No dependencies
                    Some(deps) => deps.iter().all(|dep_id| completed.contains(dep_id)),
                };

                if all_deps_met {
                    current_stage.push(*task_id);
                }
            }

            if current_stage.is_empty() {
                // No progress made - circular dependency
                return Err(TaskPlanError::CircularDependency);
            }

            // Remove current stage from remaining
            remaining.retain(|id| !current_stage.contains(id));

            // Mark as completed
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
            // Find best agent for this subtask's required capabilities
            let agent_id = if subtask.required_capabilities.len() == 1 {
                // Single capability - find best agent
                self.agent_registry
                    .find_best_agent(&subtask.required_capabilities[0])
                    .await
            } else {
                // Multiple capabilities - find agent with all capabilities
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

/// Analysis of user intent.
#[derive(Debug)]
struct IntentAnalysis {
    keywords: Vec<String>,
    entities: Vec<Entity>,
}

/// Extracted entity from intent.
#[derive(Debug)]
struct Entity {
    entity_type: String,
    value: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DevicePlatform;

    #[tokio::test]
    async fn test_intent_analysis() {
        let planner = create_test_planner().await;
        let analysis = planner.analyze_intent("Find nearby coffee shops with outdoor seating");

        assert!(analysis.keywords.contains(&"location".to_string()));
        assert!(analysis.keywords.contains(&"search".to_string()));
        assert!(analysis.keywords.contains(&"filter".to_string()));
    }

    #[tokio::test]
    async fn test_task_planning() {
        let planner = create_test_planner().await;

        let task_offer = TaskOffer {
            intent_id: Uuid::new_v4().to_string(),
            intent: "Find nearby coffee shops".to_string(),
            capabilities_hint: Some(vec!["location".to_string(), "search".to_string()]),
            device_caps: DeviceCapabilities {
                ai_capabilities: vec![],
                sensors: vec![],
                platform: DevicePlatform::Ios,
                os_version: "test".to_string(),
            },
            context: None,
        };

        let plan = planner.plan_task(task_offer).await.unwrap();

        assert!(!plan.subtasks.is_empty());
        assert!(!plan.execution_order.is_empty());
    }

    async fn create_test_planner() -> TaskPlanner {
        let registry = Arc::new(AgentRegistry::new());

        // Register test agents
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
