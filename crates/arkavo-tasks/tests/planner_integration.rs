//! Integration tests for TaskPlanner with pluggable IntentAnalyzer.
//!
//! These tests exercise the full planning pipeline: intent analysis → subtask
//! creation → dependency resolution → execution ordering, using mock analyzers
//! to simulate both rule-based and LLM-generated plans.

use arkavo_tasks::AgentRegistry;
use arkavo_tasks::intent_analyzer::{IntentAnalysis, IntentAnalyzer, SubTaskSpec};
use arkavo_tasks::task_planner::{TaskPlanError, TaskPlanner, create_subtasks_from_specs};
use async_trait::async_trait;
use std::sync::Arc;

// -- Mock analyzers ----------------------------------------------------------

struct ThreeStageAnalyzer;

#[async_trait]
impl IntentAnalyzer for ThreeStageAnalyzer {
    async fn analyze(&self, _intent: &str) -> Result<IntentAnalysis, TaskPlanError> {
        Ok(IntentAnalysis {
            keywords: vec!["search".into()],
            entities: vec![],
            subtask_specs: vec![
                SubTaskSpec {
                    task_type: "search".into(),
                    description: "Search for information".into(),
                    required_capabilities: vec!["web".into()],
                    depends_on: vec![],
                },
                SubTaskSpec {
                    task_type: "filter".into(),
                    description: "Filter results".into(),
                    required_capabilities: vec![],
                    depends_on: vec![0],
                },
                SubTaskSpec {
                    task_type: "summarize".into(),
                    description: "Summarize filtered results".into(),
                    required_capabilities: vec![],
                    depends_on: vec![1],
                },
            ],
        })
    }
}

struct FanOutFanInAnalyzer {
    fan_width: usize,
}

#[async_trait]
impl IntentAnalyzer for FanOutFanInAnalyzer {
    async fn analyze(&self, _intent: &str) -> Result<IntentAnalysis, TaskPlanError> {
        let mut specs = Vec::new();
        // Fan-out: N independent search tasks
        for i in 0..self.fan_width {
            specs.push(SubTaskSpec {
                task_type: "search".into(),
                description: format!("Search source {i}"),
                required_capabilities: vec![],
                depends_on: vec![],
            });
        }
        // Fan-in: merge depends on all searches
        let deps: Vec<usize> = (0..self.fan_width).collect();
        specs.push(SubTaskSpec {
            task_type: "merge".into(),
            description: "Merge all search results".into(),
            required_capabilities: vec![],
            depends_on: deps,
        });
        // Final summary depends on merge
        specs.push(SubTaskSpec {
            task_type: "summarize".into(),
            description: "Summarize merged results".into(),
            required_capabilities: vec![],
            depends_on: vec![self.fan_width],
        });

        Ok(IntentAnalysis {
            keywords: vec!["parallel_search".into()],
            entities: vec![],
            subtask_specs: specs,
        })
    }
}

// -- Tests -------------------------------------------------------------------

#[tokio::test]
async fn linear_chain_produces_sequential_stages() {
    let registry = Arc::new(AgentRegistry::new());
    let planner = TaskPlanner::with_analyzer(registry, Box::new(ThreeStageAnalyzer));

    let (subtasks, order, deps) = planner.plan_subtasks("anything").await.unwrap();

    assert_eq!(subtasks.len(), 3);
    assert_eq!(order.len(), 3); // strictly sequential
    for stage in &order {
        assert_eq!(stage.len(), 1);
    }

    // First stage has no deps
    assert!(!deps.contains_key(&subtasks[0].id));
    // Each subsequent stage depends on exactly one predecessor
    assert_eq!(deps[&subtasks[1].id].len(), 1);
    assert_eq!(deps[&subtasks[2].id].len(), 1);
}

#[tokio::test]
async fn fan_out_fan_in_with_three_parallel() {
    let registry = Arc::new(AgentRegistry::new());
    let planner =
        TaskPlanner::with_analyzer(registry, Box::new(FanOutFanInAnalyzer { fan_width: 3 }));

    let (subtasks, order, deps) = planner.plan_subtasks("parallel search").await.unwrap();

    assert_eq!(subtasks.len(), 5); // 3 searches + merge + summarize
    assert_eq!(order.len(), 3); // stage1: 3 searches, stage2: merge, stage3: summarize
    assert_eq!(order[0].len(), 3); // parallel stage
    assert_eq!(order[1].len(), 1); // merge
    assert_eq!(order[2].len(), 1); // summarize

    // merge depends on all 3 searches
    let merge_id = subtasks[3].id;
    assert_eq!(deps[&merge_id].len(), 3);
}

#[tokio::test]
async fn fan_out_fan_in_with_six_parallel() {
    let registry = Arc::new(AgentRegistry::new());
    let planner =
        TaskPlanner::with_analyzer(registry, Box::new(FanOutFanInAnalyzer { fan_width: 6 }));

    let (subtasks, order, _) = planner.plan_subtasks("wide search").await.unwrap();

    assert_eq!(subtasks.len(), 8); // 6 + merge + summarize
    assert_eq!(order[0].len(), 6); // all 6 in parallel
}

#[tokio::test]
async fn single_spec_produces_single_subtask() {
    struct SingleSpecAnalyzer;

    #[async_trait]
    impl IntentAnalyzer for SingleSpecAnalyzer {
        async fn analyze(&self, _intent: &str) -> Result<IntentAnalysis, TaskPlanError> {
            Ok(IntentAnalysis {
                keywords: vec![],
                entities: vec![],
                subtask_specs: vec![SubTaskSpec {
                    task_type: "answer".into(),
                    description: "Answer directly".into(),
                    required_capabilities: vec![],
                    depends_on: vec![],
                }],
            })
        }
    }

    let registry = Arc::new(AgentRegistry::new());
    let planner = TaskPlanner::with_analyzer(registry, Box::new(SingleSpecAnalyzer));

    let (subtasks, order, deps) = planner.plan_subtasks("simple").await.unwrap();

    assert_eq!(subtasks.len(), 1);
    assert_eq!(order.len(), 1);
    assert!(deps.is_empty());
}

#[tokio::test]
async fn circular_dependency_detected() {
    struct CyclicAnalyzer;

    #[async_trait]
    impl IntentAnalyzer for CyclicAnalyzer {
        async fn analyze(&self, _intent: &str) -> Result<IntentAnalysis, TaskPlanError> {
            Ok(IntentAnalysis {
                keywords: vec![],
                entities: vec![],
                subtask_specs: vec![
                    SubTaskSpec {
                        task_type: "a".into(),
                        description: "A".into(),
                        required_capabilities: vec![],
                        depends_on: vec![2], // A→C
                    },
                    SubTaskSpec {
                        task_type: "b".into(),
                        description: "B".into(),
                        required_capabilities: vec![],
                        depends_on: vec![0], // B→A
                    },
                    SubTaskSpec {
                        task_type: "c".into(),
                        description: "C".into(),
                        required_capabilities: vec![],
                        depends_on: vec![1], // C→B → cycle A→C→B→A
                    },
                ],
            })
        }
    }

    let registry = Arc::new(AgentRegistry::new());
    let planner = TaskPlanner::with_analyzer(registry, Box::new(CyclicAnalyzer));

    let err = planner.plan_subtasks("cycle").await.unwrap_err();
    assert!(
        matches!(err, TaskPlanError::CircularDependency),
        "Expected CircularDependency, got: {err}"
    );
}

#[tokio::test]
async fn analyzer_error_propagates_through_plan_subtasks() {
    struct ErrorAnalyzer;

    #[async_trait]
    impl IntentAnalyzer for ErrorAnalyzer {
        async fn analyze(&self, _intent: &str) -> Result<IntentAnalysis, TaskPlanError> {
            Err(TaskPlanError::LlmAnalysisFailed("model crashed".into()))
        }
    }

    let registry = Arc::new(AgentRegistry::new());
    let planner = TaskPlanner::with_analyzer(registry, Box::new(ErrorAnalyzer));

    let err = planner.plan_subtasks("boom").await.unwrap_err();
    assert!(
        matches!(err, TaskPlanError::LlmAnalysisFailed(ref msg) if msg == "model crashed"),
        "Expected LlmAnalysisFailed, got: {err}"
    );
}

#[test]
fn create_subtasks_from_specs_assigns_unique_uuids() {
    let specs = vec![
        SubTaskSpec {
            task_type: "a".into(),
            description: "A".into(),
            required_capabilities: vec![],
            depends_on: vec![],
        },
        SubTaskSpec {
            task_type: "b".into(),
            description: "B".into(),
            required_capabilities: vec![],
            depends_on: vec![],
        },
    ];

    let (subtasks, _) = create_subtasks_from_specs(&specs);
    assert_ne!(subtasks[0].id, subtasks[1].id);
}

#[test]
fn create_subtasks_from_specs_preserves_all_fields() {
    let specs = vec![SubTaskSpec {
        task_type: "custom_type".into(),
        description: "Do something specific".into(),
        required_capabilities: vec!["cap1".into(), "cap2".into()],
        depends_on: vec![],
    }];

    let (subtasks, _) = create_subtasks_from_specs(&specs);
    assert_eq!(subtasks[0].task_type, "custom_type");
    assert_eq!(subtasks[0].description, "Do something specific");
    assert_eq!(
        subtasks[0].required_capabilities,
        vec!["cap1".to_string(), "cap2".to_string()]
    );
}

#[tokio::test]
async fn rule_based_fallback_when_specs_empty() {
    let registry = Arc::new(AgentRegistry::new());
    let planner = TaskPlanner::new(registry); // uses RuleBasedAnalyzer

    // "Find nearby coffee" triggers the legacy keyword path
    let (subtasks, order, deps) = planner
        .plan_subtasks("Find nearby coffee shops")
        .await
        .unwrap();

    // Should produce: sensor_request, web_search, summarize
    assert!(subtasks.len() >= 3);
    assert!(subtasks.iter().any(|s| s.task_type == "sensor_request"));
    assert!(subtasks.iter().any(|s| s.task_type == "web_search"));
    assert!(subtasks.iter().any(|s| s.task_type == "summarize"));
    assert!(!order.is_empty());
    assert!(!deps.is_empty());
}

#[tokio::test]
async fn diamond_dependency_execution_order() {
    struct DiamondAnalyzer;

    #[async_trait]
    impl IntentAnalyzer for DiamondAnalyzer {
        async fn analyze(&self, _intent: &str) -> Result<IntentAnalysis, TaskPlanError> {
            Ok(IntentAnalysis {
                keywords: vec![],
                entities: vec![],
                subtask_specs: vec![
                    SubTaskSpec {
                        task_type: "root".into(),
                        description: "Root task".into(),
                        required_capabilities: vec![],
                        depends_on: vec![],
                    },
                    SubTaskSpec {
                        task_type: "left".into(),
                        description: "Left branch".into(),
                        required_capabilities: vec![],
                        depends_on: vec![0],
                    },
                    SubTaskSpec {
                        task_type: "right".into(),
                        description: "Right branch".into(),
                        required_capabilities: vec![],
                        depends_on: vec![0],
                    },
                    SubTaskSpec {
                        task_type: "join".into(),
                        description: "Join branches".into(),
                        required_capabilities: vec![],
                        depends_on: vec![1, 2],
                    },
                ],
            })
        }
    }

    let registry = Arc::new(AgentRegistry::new());
    let planner = TaskPlanner::with_analyzer(registry, Box::new(DiamondAnalyzer));

    let (subtasks, order, _) = planner.plan_subtasks("diamond").await.unwrap();

    assert_eq!(subtasks.len(), 4);
    // Stage 1: root
    // Stage 2: left + right (parallel)
    // Stage 3: join
    assert_eq!(order.len(), 3);
    assert_eq!(order[0].len(), 1); // root
    assert_eq!(order[1].len(), 2); // left + right in parallel
    assert_eq!(order[2].len(), 1); // join
}
