use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    /// A failed or refused response can still consume billable tokens.
    #[error("Provider error: {message}")]
    ProviderResponseFailure {
        message: String,
        inference_timing: Option<crate::provider::InferenceTiming>,
    },
    #[cfg(feature = "llm-remote")]
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("JSON parsing failed: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Stream error: {0}")]
    Stream(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Provider error: {0}")]
    Provider(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid image format: {0}")]
    InvalidImageFormat(String),

    #[error("Invalid image path: {0}")]
    InvalidImagePath(String),

    #[error("Model error: {0}")]
    Model(String),

    #[error("Inference error: {0}")]
    Inference(String),

    #[error("Not implemented: {0}")]
    NotImplemented(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("GPU fault ({kind}): {message}")]
    GpuFault { kind: String, message: String },
}

impl Error {
    /// Retain known billing metadata when a later validation or release check fails.
    pub fn with_inference_timing(self, timing: Option<crate::provider::InferenceTiming>) -> Self {
        if self.inference_timing().is_some() || timing.is_none() {
            return self;
        }
        Self::ProviderResponseFailure {
            message: self.to_string(),
            inference_timing: timing,
        }
    }

    pub fn inference_timing(&self) -> Option<&crate::provider::InferenceTiming> {
        match self {
            Self::ProviderResponseFailure {
                inference_timing, ..
            } => inference_timing.as_ref(),
            _ => None,
        }
    }

    /// Whether this error represents a GPU fault that may be recoverable via retry.
    pub fn is_gpu_fault(&self) -> bool {
        matches!(self, Error::GpuFault { .. })
    }

    /// Whether this error is retryable (currently only GPU faults).
    pub fn is_retryable(&self) -> bool {
        self.is_gpu_fault()
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = Error::Config("Invalid configuration".to_string());
        assert_eq!(
            err.to_string(),
            "Configuration error: Invalid configuration"
        );

        let err = Error::Stream("Connection lost".to_string());
        assert_eq!(err.to_string(), "Stream error: Connection lost");

        let err = Error::Provider("Model not found".to_string());
        assert_eq!(err.to_string(), "Provider error: Model not found");
    }

    #[test]
    #[cfg(feature = "llm-remote")]
    fn test_error_from_reqwest() {
        // Test that we can convert reqwest errors
        // Note: Creating actual reqwest errors is complex, so we test the type system
        // Verify the conversion exists at compile time
        let _: fn(reqwest::Error) -> Error = |e| e.into();
    }

    #[test]
    fn test_error_from_json() {
        let json_str = r#"{"invalid": json}"#;
        let parse_result: serde_json::Result<serde_json::Value> = serde_json::from_str(json_str);
        if let Err(json_err) = parse_result {
            let err: Error = json_err.into();
            assert!(matches!(err, Error::Json(_)));
        }
    }

    #[test]
    fn test_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "File not found");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
    }

    #[test]
    fn test_result_type() {
        fn returns_result() -> Result<String> {
            Ok("success".to_string())
        }

        fn returns_error() -> Result<String> {
            Err(Error::Config("test error".to_string()))
        }

        assert!(returns_result().is_ok());
        assert!(returns_error().is_err());
    }

    #[test]
    fn test_gpu_fault_display() {
        let err = Error::GpuFault {
            kind: "MetalKill".to_string(),
            message: "code: -3".to_string(),
        };
        assert_eq!(err.to_string(), "GPU fault (MetalKill): code: -3");
    }

    #[test]
    fn test_gpu_fault_is_retryable() {
        let gpu = Error::GpuFault {
            kind: "MetalKill".to_string(),
            message: "test".to_string(),
        };
        assert!(gpu.is_gpu_fault());
        assert!(gpu.is_retryable());
    }

    #[test]
    fn test_non_gpu_not_retryable() {
        let config = Error::Config("test".to_string());
        assert!(!config.is_gpu_fault());
        assert!(!config.is_retryable());

        let inference = Error::Inference("test".to_string());
        assert!(!inference.is_retryable());
    }
}
