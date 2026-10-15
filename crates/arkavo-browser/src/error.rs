use thiserror::Error;

#[derive(Error, Debug)]
pub enum BrowserError {
    #[error("Playwright error: {0}")]
    Playwright(String),

    #[error("Page navigation failed: {0}")]
    Navigation(String),

    #[error("Element not found: {0}")]
    ElementNotFound(String),

    #[error("Screenshot failed: {0}")]
    Screenshot(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Environment error: {0}")]
    Environment(String),

    #[error("Invalid parameters: {0}")]
    InvalidParams(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, BrowserError>;

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    /// Every variant keeps the context it was created with in its Display
    /// output, so a propagated failure still says what was being attempted.
    #[spec("BROWS-006")]
    #[test]
    fn test_every_variant_preserves_its_context() {
        let cases: Vec<(BrowserError, &str)> = vec![
            (BrowserError::Playwright("launch ctx".into()), "launch ctx"),
            (BrowserError::Navigation("nav ctx".into()), "nav ctx"),
            (BrowserError::ElementNotFound("#btn".into()), "#btn"),
            (BrowserError::Screenshot("shot ctx".into()), "shot ctx"),
            (BrowserError::Network("net ctx".into()), "net ctx"),
            (BrowserError::Timeout("30s".into()), "30s"),
            (BrowserError::Environment("env ctx".into()), "env ctx"),
            (BrowserError::InvalidParams("param ctx".into()), "param ctx"),
        ];
        for (err, context) in cases {
            let shown = err.to_string();
            assert!(shown.contains(context), "{shown} missing {context:?}");
        }
    }

    /// Lower-level failures convert into BrowserError without losing the
    /// source error's message.
    #[spec("BROWS-006")]
    #[test]
    fn test_io_and_json_errors_convert_with_context_preserved() {
        let io = BrowserError::from(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "screenshot path missing",
        ));
        assert!(matches!(io, BrowserError::Io(_)), "{io:?}");
        assert!(io.to_string().contains("screenshot path missing"), "{io}");

        let json =
            BrowserError::from(serde_json::from_str::<serde_json::Value>("{bad json").unwrap_err());
        assert!(matches!(json, BrowserError::Json(_)), "{json:?}");
        assert!(!json.to_string().is_empty());
    }
}
