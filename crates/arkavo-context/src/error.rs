use std::fmt;

#[derive(Debug)]
pub enum Error {
    Compression(String),
    Model(String),
    Config(String),
    InvalidFormat(String),
    ChunkNotFound(String),
    StorageError(String),
    TokenizerError(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compression(msg) => write!(f, "Compression error: {msg}"),
            Self::Model(msg) => write!(f, "Model error: {msg}"),
            Self::Config(msg) => write!(f, "Configuration error: {msg}"),
            Self::InvalidFormat(msg) => write!(f, "Invalid format: {msg}"),
            Self::ChunkNotFound(msg) => write!(f, "Chunk not found: {msg}"),
            Self::StorageError(msg) => write!(f, "Storage error: {msg}"),
            Self::TokenizerError(msg) => write!(f, "Tokenizer error: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(any(feature = "llama-cpp", feature = "llm-remote"))]
impl From<arkavo_llm::Error> for Error {
    fn from(e: arkavo_llm::Error) -> Self {
        Self::Model(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_compression() {
        let e = Error::Compression("failed".to_string());
        assert_eq!(e.to_string(), "Compression error: failed");
    }

    #[test]
    fn test_display_model() {
        let e = Error::Model("bad model".to_string());
        assert_eq!(e.to_string(), "Model error: bad model");
    }

    #[test]
    fn test_display_config() {
        let e = Error::Config("missing key".to_string());
        assert_eq!(e.to_string(), "Configuration error: missing key");
    }

    #[test]
    fn test_display_invalid_format() {
        let e = Error::InvalidFormat("wrong type".to_string());
        assert_eq!(e.to_string(), "Invalid format: wrong type");
    }

    #[test]
    fn test_display_chunk_not_found() {
        let e = Error::ChunkNotFound("id-42".to_string());
        assert_eq!(e.to_string(), "Chunk not found: id-42");
    }

    #[test]
    fn test_display_storage_error() {
        let e = Error::StorageError("disk full".to_string());
        assert_eq!(e.to_string(), "Storage error: disk full");
    }

    #[test]
    fn test_display_tokenizer_error() {
        let e = Error::TokenizerError("unknown token".to_string());
        assert_eq!(e.to_string(), "Tokenizer error: unknown token");
    }
}
