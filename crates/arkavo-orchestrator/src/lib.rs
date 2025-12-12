#![allow(clippy::future_not_send)]
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::unused_async)]
#![allow(clippy::needless_continue)]
#![allow(clippy::large_enum_variant)]

pub mod agent_assignment;
pub mod code_solver;
pub mod cognitive_engine;
mod cognitive_engine_core;
mod cognitive_engine_planning;
mod cognitive_engine_pr;
mod cognitive_engine_verification;
pub mod config;
pub mod error;
pub mod github_auth;
pub mod github_operations;
pub mod issue_analyzer;
pub mod issue_router;
pub mod oidc;
pub mod orchestrator;
pub mod types;
pub mod webhook;

pub use agent_assignment::{AgentAssigner, AgentAssignment};
pub use code_solver::{CodeSolver, SolverConfig, SolverMetrics, SolverResult};
pub use cognitive_engine::{CognitiveEngine, ExecutionPlan, ExecutionResult};
pub use config::OrchestratorConfig;
pub use error::{Error, Result};
pub use github_auth::GitHubApp;
pub use github_operations::{GitHubOperations, IssueUpdate};
pub use issue_analyzer::{Complexity, IssueAnalysis, IssueAnalyzer, IssueType};
pub use issue_router::{ExecutionStrategy, IssueRouter, Priority, RoutingDecision};
pub use orchestrator::Orchestrator;
pub use types::{GitHubEvent, IssueEvent, PullRequestEvent};
pub use webhook::WebhookServer;
