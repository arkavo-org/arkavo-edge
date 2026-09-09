//! Stateless OpenAI Responses transport for GPT-6 Astra.
//!
//! Encrypted reasoning items belong to the caller's transcript, never a global
//! response-id cache. This preserves tool continuations across provider instances
//! without mixing concurrent sessions or retaining responses on the service.
mod config;
mod convert;
mod provider;
mod schema;
mod sse;

pub use config::{OpenAIReasoningEffort, OpenAIResponsesConfig};
pub use provider::OpenAIResponsesProvider;
