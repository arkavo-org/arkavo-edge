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
    Budget(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Error>;
