use thiserror::Error;

pub type Result<T> = std::result::Result<T, SnpeError>;

#[derive(Error, Debug)]
pub enum SnpeError {
    #[error(
        "SNPE SDK not found. Set SNPE_ROOT environment variable to SDK installation directory."
    )]
    SdkNotFound,

    #[error("Failed to load SNPE library: {0}")]
    LibraryLoadFailed(String),

    #[error("Invalid SNPE SDK path: {0}")]
    InvalidSdkPath(String),

    #[error("SNPE SDK version mismatch. Expected {expected}, found {found}")]
    VersionMismatch { expected: String, found: String },

    #[error("Failed to initialize SNPE runtime: {0}")]
    RuntimeInit(String),

    #[error("Failed to load DLC model from {path}: {reason}")]
    ModelLoad { path: String, reason: String },

    #[error("Invalid DLC file format: {0}")]
    InvalidDlcFormat(String),

    #[error("Requested accelerator {0} is not available on this device")]
    AcceleratorUnavailable(String),

    #[error("Tensor shape mismatch. Expected {expected:?}, got {actual:?}")]
    TensorShapeMismatch {
        expected: Vec<usize>,
        actual: Vec<usize>,
    },

    #[error("Invalid tensor data type: {0}")]
    InvalidTensorType(String),

    #[error("Inference execution failed: {0}")]
    InferenceFailed(String),

    #[error("Memory allocation failed: {0}")]
    MemoryAllocation(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Unsupported operation: {0}")]
    Unsupported(String),

    #[error("SNPE internal error: {0}")]
    Internal(String),
}

impl SnpeError {
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            SnpeError::AcceleratorUnavailable(_)
                | SnpeError::MemoryAllocation(_)
                | SnpeError::InferenceFailed(_)
        )
    }

    pub fn with_context<S: Into<String>>(self, context: S) -> Self {
        match self {
            SnpeError::Internal(msg) => SnpeError::Internal(format!("{}: {}", context.into(), msg)),
            other => other,
        }
    }
}
