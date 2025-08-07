pub mod cache;
pub mod client;
pub mod config;
pub mod error;
pub mod types;

pub use client::AuthorizationClient;
pub use config::AuthorizationConfig;
pub use error::{AuthorizationError, Result};
pub use types::{
    Action, Decision, DecisionRequest, EntityIdentifier, GetDecisionBulkRequest,
    GetDecisionBulkResponse, GetDecisionMultiResourceRequest, GetDecisionMultiResourceResponse,
    GetDecisionRequest, GetDecisionResponse, Resource,
};

#[cfg(test)]
mod tests;
