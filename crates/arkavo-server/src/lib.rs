#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::significant_drop_in_scrutinee)]

//! # Arkavo Server
//!
//! A2A server implementation for the Arkavo agentic CLI tool.
//!
//! This crate provides the JSON-RPC server, handlers, and supporting infrastructure
//! for A2A protocol communication.

pub mod server;

// Re-export commonly used types
pub use server::{
    A2aServer, AgentGoal, AgentMetadata, AgentPlan, BehaviorAdvice, EpisodeBuffer, GoalStatus,
    LearningBus, LearningConfig, LearningEvent, McpBridgeTool, PolicyCache, RlmBridge, ToolMemory,
    ToolMemoryEntry, ToolObservation, ToolPatternCache, ToolPatternObserver, WellKnownState,
    estimate_tokens, execute_with_conductor, execute_with_conductor_and_learning,
    model_context_size, run_startup_planning_phase, start_anti_entropy_loop, start_cleanup_loop,
    start_event_processing_loop, start_gossip_transport, start_lesson_application_loop,
    start_lesson_propagation_loop, start_well_known_server,
};
