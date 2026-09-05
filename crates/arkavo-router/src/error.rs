use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Classification error: {0}")]
    Classification(String),

    #[error("Model selection error: {0}")]
    Selection(String),

    #[error(
        "Cloud inference with {model} requires confirmation (estimated ${estimated_cost_usd:.4}). Select --model {model} to explicitly authorize this model."
    )]
    CloudConfirmationRequired {
        model: String,
        estimated_cost_usd: f64,
    },

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

    #[error("Context decomposition error: {0}")]
    ContextDecomposition(String),

    #[error("Missing tool use: suggested keywords {keywords:?}")]
    MissingToolUse { keywords: Vec<String> },

    #[error("Manifest not found: {0}")]
    ManifestNotFound(String),

    /// Response rejected by CriticPipeline
    #[cfg(feature = "critic")]
    #[error("Response rejected by critic: {failures:?}")]
    CriticRejected { failures: Vec<String> },
}

impl Error {
    /// Whether the underlying cause is a GPU fault (Metal InnocentVictim, etc.)
    pub fn is_gpu_fault(&self) -> bool {
        matches!(self, Error::Provider(e) if e.is_gpu_fault())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpu_fault_propagates_through_provider() {
        let llm_err = arkavo_llm::Error::GpuFault {
            kind: "MetalKill".to_string(),
            message: "code: -3".to_string(),
        };
        let router_err = Error::Provider(llm_err);
        assert!(router_err.is_gpu_fault());
    }

    #[test]
    fn test_non_gpu_provider_error() {
        let llm_err = arkavo_llm::Error::Config("test".to_string());
        let router_err = Error::Provider(llm_err);
        assert!(!router_err.is_gpu_fault());
    }

    #[test]
    fn test_non_provider_error_not_gpu_fault() {
        let err = Error::Config("test".to_string());
        assert!(!err.is_gpu_fault());
    }
}
