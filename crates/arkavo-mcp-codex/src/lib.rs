//! Rust bridge to Codex's non-interactive JSONL protocol.
//! The host grants a workspace and spend authority; prompts cannot widen either.
// The modules holding process and protocol internals are declared `pub` so
// that the `pub(crate)` on their items expresses a real visibility boundary
// rather than a redundant one inside a private module. Nothing new becomes
// reachable: every internal item stays `pub(crate)`, and the API of this crate
// is exactly the re-exports below.
mod config;
pub mod containment;
pub mod events;
pub mod process;
pub mod store;
#[cfg(feature = "mcp-tools")]
mod tools;
mod worker;

pub use config::{CodexConfig, Sandbox, SpendApproval};
pub use events::{FileChange, RunOutcome, RunStatus, Usage};
pub use store::SessionBinding;
#[cfg(feature = "mcp-tools")]
pub use tools::register_tools;
pub use worker::CodexWorker;
