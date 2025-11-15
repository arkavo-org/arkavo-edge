use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Encryption error: {0}")]
    Encryption(String),

    #[error("Decryption error: {0}")]
    Decryption(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Deserialization error: {0}")]
    Deserialization(String),

    #[error("Authorization error: {0}")]
    Authorization(String),

    #[error("Policy error: {0}")]
    Policy(String),

    #[error("Key generation error: {0}")]
    KeyGeneration(String),

    #[error("Signature error: {0}")]
    Signature(String),
}

pub type Result<T> = std::result::Result<T, Error>;
