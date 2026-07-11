//! xAI Responses API client (`POST /v1/responses`).
//!
//! Preferred path for Grok 4.5 agentic work. Uses the Responses surface (not
//! Chat Completions) for configurable reasoning effort, function-call items,
//! and optional SSE streaming.
//!
//! ## Multi-turn (v1)
//!
//! The standard [`crate::Provider`] path re-sends the full transcript each
//! turn. Server-side `previous_response_id` chaining is available via
//! [`ResponsesProvider::continue_with_tool_outputs`] when `store` is enabled.
//! Chat Completions remains available through
//! [`super::openai::OpenAIProvider`] for OpenAI-compatible hosts.

mod config;
mod convert;
mod provider;
mod sse;
mod types;

pub use config::{ReasoningEffort, ResponsesConfig};
pub use provider::ResponsesProvider;
pub use types::ResponsesResult;
