use std::fmt;

#[derive(Debug)]
pub enum Error {
    Compression(String),
    Model(String),
    Config(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compression(msg) => write!(f, "Compression error: {msg}"),
            Self::Model(msg) => write!(f, "Model error: {msg}"),
            Self::Config(msg) => write!(f, "Configuration error: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<arkavo_llm::Error> for Error {
    fn from(e: arkavo_llm::Error) -> Self {
        Self::Model(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
