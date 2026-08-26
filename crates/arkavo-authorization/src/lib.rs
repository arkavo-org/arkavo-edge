pub mod cache;
pub mod client;
pub mod config;
pub(crate) mod cwt_decode;
pub mod cwt_subject;
pub mod cwt_verify;
pub mod error;
pub mod pep;
pub mod types;

pub use client::AuthorizationClient;
pub use config::AuthorizationConfig;
pub use error::{AuthorizationError, Result, jsonrpc_codes};
pub use pep::{is_hardcoded_mapped, is_pass_through, subject_cwt_from};
pub use types::{Action, Decision, McpToolMapping, Resource};

#[cfg(test)]
mod tests;
