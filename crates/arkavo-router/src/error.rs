use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Classification error: {0}")]
    Classification(String),

    #[error("Model selection error: {0}")]
    Selection(String),

    #[error("Cost estimation error: {0}")]
    CostEstimation(String),

    #[error("LLM provider error: {0}")]
    Provider(#[from] arkavo_llm::Error),

    #[error("Budget error: {0}")]
    BudgetError(String),

    #[error("Budget exceeded: {0}")]
    BudgetExceeded(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Model execution failed: {0}")]
    ModelExecution(String),

    #[error("Response validation failed: {0}")]
    Validation(#[from] crate::validator::ValidationError),

    #[error("Max retries exceeded after {attempts} attempts")]
    MaxRetriesExceeded { attempts: u8 },

    #[error("Architect mode error: {0}")]
    ArchitectError(String),

    #[error("Request blocked by policy '{policy_id}': {reason}")]
    ModerationBlocked { policy_id: String, reason: String },

    /// Response rejected by CriticPipeline
    #[cfg(feature = "critic")]
    #[error("Response rejected by critic: {failures:?}")]
    CriticRejected { failures: Vec<String> },
}

pub type Result<T> = std::result::Result<T, Error>;
