use thiserror::Error;

/// Errors that can occur during wallet operations
#[derive(Error, Debug)]
pub enum WalletError {
    #[error("Invalid key format: {0}")]
    InvalidKeyFormat(String),

    #[error("Invalid signature")]
    InvalidSignature,

    #[error("Signature verification failed")]
    VerificationFailed,

    #[error("Invalid mnemonic: {0}")]
    InvalidMnemonic(String),

    #[error("Invalid derivation path: {0}")]
    InvalidDerivationPath(String),

    #[error("Invalid address format: {0}")]
    InvalidAddress(String),

    #[error("Invalid transaction: {0}")]
    InvalidTransaction(String),

    #[error("RLP encoding error: {0}")]
    RlpEncodingError(String),

    #[error("Cryptographic operation failed: {0}")]
    CryptoError(String),
}

pub type Result<T> = std::result::Result<T, WalletError>;
