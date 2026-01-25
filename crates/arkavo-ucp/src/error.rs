use thiserror::Error;

/// Errors that can occur during UCP operations
#[derive(Error, Debug)]
pub enum UcpError {
    #[error("Payment creation failed: {0}")]
    PaymentCreationFailed(String),

    #[error("Payment execution failed: {0}")]
    PaymentExecutionFailed(String),

    #[error("Policy violation: {0}")]
    PolicyViolation(String),

    #[error("Invalid payment intent: {0}")]
    InvalidPaymentIntent(String),

    #[error("Merchant not found: {0}")]
    MerchantNotFound(String),

    #[error("Unsupported currency: {0}")]
    UnsupportedCurrency(String),

    #[error("Payment not found: {0}")]
    PaymentNotFound(String),

    #[error("Wallet error: {0}")]
    WalletError(#[from] arkavo_wallet::WalletError),

    #[error("Budget error: {0}")]
    BudgetError(String),

    #[error("Handler not available: {0}")]
    HandlerNotAvailable(String),

    #[error("Invalid amount: {0}")]
    InvalidAmount(String),
}

pub type Result<T> = std::result::Result<T, UcpError>;
