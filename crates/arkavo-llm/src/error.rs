use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
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
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = Error::Config("Invalid configuration".to_string());
        assert_eq!(err.to_string(), "Configuration error: Invalid configuration");

        let err = Error::Stream("Connection lost".to_string());
        assert_eq!(err.to_string(), "Stream error: Connection lost");

        let err = Error::Provider("Model not found".to_string());
        assert_eq!(err.to_string(), "Provider error: Model not found");
    }

    #[test]
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
}