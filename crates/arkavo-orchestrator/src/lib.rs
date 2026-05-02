#![allow(clippy::future_not_send)]
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::unused_async)]
#![allow(clippy::needless_continue)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::unnecessary_literal_bound)]
#![allow(clippy::or_fun_call)]
#![allow(clippy::redundant_clone)]
#![allow(clippy::struct_excessive_bools)]

pub mod agent_assignment;
pub mod attempt_history;
pub mod chunk_processor;
pub mod code_solver;
pub mod cognitive_engine;
mod cognitive_engine_core;
mod cognitive_engine_helpers;
mod cognitive_engine_outcome;
mod cognitive_engine_planning;
mod cognitive_engine_planning_parser;
mod cognitive_engine_pr;
pub mod cognitive_engine_schema;
mod cognitive_engine_step;
mod cognitive_engine_timing;
mod cognitive_engine_types;
mod cognitive_engine_verification;
pub mod config;
pub mod error;
pub mod issue_analyzer;
pub mod issue_router;
pub mod orchestrator;
pub mod plan_validator;
pub mod planner_config;
pub mod policy_events;
pub mod spawn_guard;
pub mod step_context;
pub mod task_executor;
pub mod task_policy_manager;
pub mod text_utils;
pub mod token_estimator;
pub mod types;
pub mod webhook;

pub use agent_assignment::{AgentAssigner, AgentAssignment};
pub use attempt_history::{AttemptHistory, AttemptRecord, FailureKind};
pub use chunk_processor::{
    AgentCost, ChunkBatch, ChunkProcessor, ChunkProcessorConfig, ChunkResult, ProcessingResult,
};
pub use code_solver::{CodeSolver, SolverConfig, SolverMetrics, SolverResult};
pub use cognitive_engine::{CognitiveEngine, ExecutionPlan, ExecutionResult};
pub use config::OrchestratorConfig;
pub use error::{Error, Result};
pub use plan_validator::{ContractViolation, ValidationReport, validate as validate_plan};
pub use step_context::StepTrace;
pub use token_estimator::{estimate_tokens, tokens_from_response, tokens_from_texts};
// Re-export GitHub types from arkavo-github for backward compatibility
pub use arkavo_github::{GitHubApp, IssueOperations, IssueUpdate};
pub use issue_analyzer::{Complexity, IssueAnalysis, IssueAnalyzer, IssueType};
pub use issue_router::{ExecutionStrategy, IssueRouter, Priority, RoutingDecision};
pub use orchestrator::Orchestrator;
pub use policy_events::{PolicyEvent, PolicyEventHandler, PolicyEventStream, StreamStats};
pub use spawn_guard::{SpawnCheckResult, SpawnGuard};
pub use task_executor::{
    CollaborativePlanner, LocalTaskStrategy, MeshTaskStrategy, MessageType, MockUI,
    ModelCapability, ModelInfo, SelectedModels, TaskConfig, TaskExecutor, TaskPlan, TaskResult,
    TaskStrategy, TaskUI,
};
pub use task_policy_manager::{
    Obligation, PolicyDecision, SandboxConfig, TaskContext, TaskPolicyManager, ToolEntitlement,
    ToolRisk,
};
pub use types::{GitHubEvent, IssueEvent, PullRequestEvent};
pub use webhook::WebhookServer;
