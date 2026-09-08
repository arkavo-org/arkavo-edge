use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    /// A failed or refused response can still consume billable tokens.
    #[error("Provider error: {message}")]
    ProviderResponseFailure {
        message: String,
        inference_timing: Option<crate::provider::InferenceTiming>,
    },
    /// A provider refused, filtered or truncated the request and named a
    /// machine-readable reason. Boxed: every `Result` in the crate pays for the
    /// largest variant.
    #[error("{0}")]
    ProviderRefusal(Box<ProviderRefusal>),

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

/// Why a provider refused, in the provider's own vocabulary.
///
/// Only identifiers are kept: a provider's `message` text can quote the prompt
/// or a credential, so it never reaches an error string or a log line. Callers
/// key retries off [`Self::code`] — a `max_output_tokens` truncation is worth
/// another attempt with a larger cap, `content_filter` and `invalid_api_key`
/// never are.
#[derive(Debug)]
pub struct ProviderRefusal {
    code: String,
    kind: Option<String>,
    inference_timing: Option<crate::provider::InferenceTiming>,
}

impl ProviderRefusal {
    /// The reason, e.g. `max_output_tokens`, `content_filter`,
    /// `insufficient_quota`, `rate_limit_exceeded`, `invalid_api_key`.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// The provider's broader class for the refusal, when it names one that
    /// the code does not already say.
    pub fn kind(&self) -> Option<&str> {
        self.kind.as_deref()
    }
}

impl std::fmt::Display for ProviderRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Provider refused: {}", self.code)?;
        match &self.kind {
            Some(kind) => write!(f, " ({kind})"),
            None => Ok(()),
        }
    }
}

impl Error {
    /// Build a refusal from wire fields, keeping the structured reason and
    /// discarding everything else the provider sent.
    ///
    /// `code` and `kind` are the provider's own identifiers (OpenAI's
    /// `error.code` and `error.type`, or an `incomplete_details.reason`);
    /// `fallback` is the caller's own identifier for the case where the
    /// provider named none. Values that are not short identifiers are prose or
    /// an echoed prompt, so they are dropped rather than surfaced.
    pub fn provider_refusal(code: Option<&str>, kind: Option<&str>, fallback: &str) -> Self {
        let code = wire_identifier(code).unwrap_or_else(|| fallback.to_string());
        Self::ProviderRefusal(Box::new(ProviderRefusal {
            kind: wire_identifier(kind).filter(|kind| *kind != code),
            code,
            inference_timing: None,
        }))
    }

    /// The provider's machine-readable refusal reason, when it named one.
    pub fn provider_code(&self) -> Option<&str> {
        match self {
            Self::ProviderRefusal(refusal) => Some(refusal.code()),
            _ => None,
        }
    }

    /// Retain known billing metadata when a later validation or release check fails.
    pub fn with_inference_timing(self, timing: Option<crate::provider::InferenceTiming>) -> Self {
        if self.inference_timing().is_some() || timing.is_none() {
            return self;
        }
        match self {
            // A refusal keeps its structured reason; only the usage was missing.
            Self::ProviderRefusal(mut refusal) => {
                refusal.inference_timing = timing;
                Self::ProviderRefusal(refusal)
            }
            other => Self::ProviderResponseFailure {
                message: other.to_string(),
                inference_timing: timing,
            },
        }
    }

    pub fn inference_timing(&self) -> Option<&crate::provider::InferenceTiming> {
        match self {
            Self::ProviderResponseFailure {
                inference_timing, ..
            } => inference_timing.as_ref(),
            Self::ProviderRefusal(refusal) => refusal.inference_timing.as_ref(),
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

/// Accept a wire code only when it is a short identifier.
///
/// Providers are not obliged to keep these fields machine-readable, and a body
/// that puts prose — or an echoed prompt — where a code belongs must not leak
/// through the one field that is allowed to be reported.
fn wire_identifier(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    is_wire_identifier(value).then(|| value.to_string())
}

fn is_wire_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// The first candidate that is a usable identifier.
///
/// Providers fill these fields inconsistently: a truncation may describe itself
/// in prose under `incomplete_details.reason` while the `error.code` beside it
/// is clean. Judging each candidate on its own keeps the clean one instead of
/// letting the first unusable value collapse the whole chain to a fallback.
pub(crate) fn first_wire_code<'a>(candidates: &[Option<&'a str>]) -> Option<&'a str> {
    candidates
        .iter()
        .flatten()
        .copied()
        .find(|value| is_wire_identifier(value.trim()))
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
    fn refusal_keeps_identifiers_and_drops_prose() {
        let refusal =
            Error::provider_refusal(Some("rate_limit_exceeded"), Some("requests"), "http_429");
        assert_eq!(refusal.provider_code(), Some("rate_limit_exceeded"));
        assert_eq!(
            refusal.to_string(),
            "Provider refused: rate_limit_exceeded (requests)"
        );
        let Error::ProviderRefusal(detail) = &refusal else {
            panic!("provider_refusal must build a refusal");
        };
        assert_eq!(detail.kind(), Some("requests"));

        // Prose in a code field is an echoed prompt risk, not a code.
        let prose = Error::provider_refusal(
            Some("Your prompt 'secret value' was rejected"),
            None,
            "http_400",
        );
        assert_eq!(prose.provider_code(), Some("http_400"));
        assert!(!prose.to_string().contains("secret"));

        // A repeated type adds nothing to the code it already names.
        let same = Error::provider_refusal(
            Some("insufficient_quota"),
            Some("insufficient_quota"),
            "http_429",
        );
        assert_eq!(same.to_string(), "Provider refused: insufficient_quota");
    }

    /// A provider that describes a truncation in prose still reports a clean
    /// code beside it; the prose must not cost us the code.
    #[test]
    fn a_prose_candidate_does_not_discard_the_clean_one_behind_it() {
        assert_eq!(
            first_wire_code(&[
                Some("The response was cut off after 'secret value'"),
                Some("max_output_tokens"),
                Some("invalid_request_error"),
            ]),
            Some("max_output_tokens")
        );
        assert_eq!(
            first_wire_code(&[None, Some("  content_filter  ")]),
            Some("  content_filter  "),
            "trimming belongs to the constructor, not the selection"
        );
        assert_eq!(
            first_wire_code(&[None, Some("a sentence, not a code")]),
            None
        );
    }

    /// Every fallible call in the crate returns `Result<_, Error>`, and clippy
    /// rejects an error type over 128 bytes. A refusal carries its detail
    /// behind a box so adding one does not widen every result in the crate.
    #[test]
    fn error_stays_within_the_result_size_budget() {
        let size = std::mem::size_of::<Error>();
        assert!(size <= 128, "Error grew to {size} bytes");
    }

    #[test]
    fn refusal_retains_its_code_when_usage_is_attached_later() {
        let timing = crate::provider::InferenceTiming {
            n_prompt_eval: 7,
            ..Default::default()
        };
        let refusal = Error::provider_refusal(Some("content_filter"), None, "incomplete")
            .with_inference_timing(Some(timing));
        assert_eq!(refusal.provider_code(), Some("content_filter"));
        assert_eq!(refusal.inference_timing().unwrap().n_prompt_eval, 7);
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
