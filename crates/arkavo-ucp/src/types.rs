use crate::error::{Result, UcpError};
use alloy_primitives::U256;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Supported currencies for UCP payments
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    /// US Dollar (fiat, tracked via budget system)
    Usd,
    /// Ethereum (native token)
    Eth,
    /// USD Coin (ERC-20 stablecoin)
    Usdc,
    /// Tether (ERC-20 stablecoin)
    Usdt,
}

impl Currency {
    /// Get decimal places for this currency
    pub fn decimals(&self) -> u8 {
        match self {
            Self::Usd => 2,
            Self::Eth => 18,
            Self::Usdc | Self::Usdt => 6,
        }
    }

    /// Check if this is a fiat currency
    pub fn is_fiat(&self) -> bool {
        matches!(self, Self::Usd)
    }

    /// Check if this is a native blockchain token
    pub fn is_native(&self) -> bool {
        matches!(self, Self::Eth)
    }

    /// Check if this is an ERC-20 token
    pub fn is_erc20(&self) -> bool {
        matches!(self, Self::Usdc | Self::Usdt)
    }
}

impl std::fmt::Display for Currency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usd => write!(f, "USD"),
            Self::Eth => write!(f, "ETH"),
            Self::Usdc => write!(f, "USDC"),
            Self::Usdt => write!(f, "USDT"),
        }
    }
}

/// Payment amount with currency
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentAmount {
    /// Amount in smallest unit (cents for USD, wei for ETH, etc.)
    pub value: u64,
    pub currency: Currency,
}

impl PaymentAmount {
    /// Create amount from cents (for USD)
    pub fn from_cents(cents: u64) -> Self {
        Self {
            value: cents,
            currency: Currency::Usd,
        }
    }

    /// Create amount from dollars
    pub fn from_dollars(dollars: f64) -> Self {
        Self {
            value: (dollars * 100.0) as u64,
            currency: Currency::Usd,
        }
    }

    /// Create amount in wei
    pub fn from_wei(wei: u64) -> Self {
        Self {
            value: wei,
            currency: Currency::Eth,
        }
    }

    /// Create amount in ether
    ///
    /// Note: For values > ~18.4 ETH, use `from_wei()` directly to avoid overflow.
    /// This method uses f64 which has precision limitations for very large values.
    pub fn from_ether(ether: f64) -> Self {
        // Saturate at u64::MAX to prevent overflow
        let wei = (ether * 1e18).min(u64::MAX as f64) as u64;
        Self {
            value: wei,
            currency: Currency::Eth,
        }
    }

    /// Get value in human-readable units
    pub fn display_value(&self) -> f64 {
        let divisor = 10f64.powi(self.currency.decimals() as i32);
        self.value as f64 / divisor
    }

    /// Convert to Wei for EVM transactions
    pub fn to_wei(&self) -> Result<U256> {
        match self.currency {
            Currency::Eth => Ok(U256::from(self.value)),
            Currency::Usdc | Currency::Usdt => Ok(U256::from(self.value)),
            Currency::Usd => Err(UcpError::UnsupportedCurrency(
                "Cannot convert USD to wei directly".to_string(),
            )),
        }
    }
}

impl std::fmt::Display for PaymentAmount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.2} {}", self.display_value(), self.currency)
    }
}

/// Merchant information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Merchant {
    pub id: String,
    pub name: String,
    pub crypto_address: Option<String>,
    pub supported_currencies: Vec<Currency>,
}

impl Merchant {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            crypto_address: None,
            supported_currencies: vec![Currency::Usd],
        }
    }

    pub fn with_crypto_address(mut self, address: impl Into<String>) -> Self {
        self.crypto_address = Some(address.into());
        self.supported_currencies.push(Currency::Eth);
        self
    }

    pub fn supports_currency(&self, currency: Currency) -> bool {
        self.supported_currencies.contains(&currency)
    }

    pub fn crypto_address(&self) -> Result<arkavo_wallet::EvmAddress> {
        let addr = self.crypto_address.as_ref().ok_or_else(|| {
            UcpError::MerchantNotFound("Merchant has no crypto address".to_string())
        })?;
        arkavo_wallet::EvmAddress::from_hex(addr).map_err(UcpError::from)
    }
}

/// Payment intent representing a request to make a payment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentIntent {
    pub id: Uuid,
    pub agent_id: String,
    pub amount: PaymentAmount,
    pub merchant: Merchant,
    pub description: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub status: PaymentStatus,
}

impl PaymentIntent {
    pub fn new(agent_id: impl Into<String>, amount: PaymentAmount, merchant: Merchant) -> Self {
        Self {
            id: Uuid::new_v4(),
            agent_id: agent_id.into(),
            amount,
            merchant,
            description: None,
            metadata: None,
            created_at: Utc::now(),
            expires_at: None,
            status: PaymentStatus::Pending,
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn with_expiry(mut self, expires_at: DateTime<Utc>) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    pub fn is_expired(&self) -> bool {
        self.expires_at.map(|e| Utc::now() > e).unwrap_or(false)
    }
}

/// Payment status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentStatus {
    /// Payment intent created, awaiting execution
    Pending,
    /// Payment is being processed
    Processing,
    /// Transaction signed but not broadcast
    Signed,
    /// Transaction broadcast, awaiting confirmation
    Broadcast,
    /// Payment completed successfully
    Completed,
    /// Payment failed
    Failed,
    /// Payment cancelled
    Cancelled,
    /// Payment expired
    Expired,
}

impl PaymentStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Expired
        )
    }
}

/// Result of a payment execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentResult {
    pub intent_id: Uuid,
    pub status: PaymentStatus,
    pub signed_tx: Option<Vec<u8>>,
    pub tx_hash: Option<String>,
    pub error: Option<String>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl PaymentResult {
    pub fn success(intent_id: Uuid) -> Self {
        Self {
            intent_id,
            status: PaymentStatus::Completed,
            signed_tx: None,
            tx_hash: None,
            error: None,
            completed_at: Some(Utc::now()),
        }
    }

    pub fn signed(intent_id: Uuid, signed_tx: Vec<u8>) -> Self {
        Self {
            intent_id,
            status: PaymentStatus::Signed,
            signed_tx: Some(signed_tx),
            tx_hash: None,
            error: None,
            completed_at: None,
        }
    }

    pub fn failed(intent_id: Uuid, error: impl Into<String>) -> Self {
        Self {
            intent_id,
            status: PaymentStatus::Failed,
            signed_tx: None,
            tx_hash: None,
            error: Some(error.into()),
            completed_at: Some(Utc::now()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_currency_decimals() {
        assert_eq!(Currency::Usd.decimals(), 2);
        assert_eq!(Currency::Eth.decimals(), 18);
        assert_eq!(Currency::Usdc.decimals(), 6);
    }

    #[test]
    fn test_currency_types() {
        assert!(Currency::Usd.is_fiat());
        assert!(Currency::Eth.is_native());
        assert!(Currency::Usdc.is_erc20());
    }

    #[test]
    fn test_payment_amount_from_cents() {
        let amount = PaymentAmount::from_cents(1000);
        assert_eq!(amount.value, 1000);
        assert_eq!(amount.display_value(), 10.0);
    }

    #[test]
    fn test_payment_amount_from_dollars() {
        let amount = PaymentAmount::from_dollars(25.50);
        assert_eq!(amount.value, 2550);
    }

    #[test]
    fn test_payment_amount_display() {
        let amount = PaymentAmount::from_dollars(99.99);
        assert_eq!(format!("{amount}"), "99.99 USD");
    }

    #[test]
    fn test_merchant_supports_currency() {
        let merchant = Merchant::new("test", "Test Merchant");
        assert!(merchant.supports_currency(Currency::Usd));
        assert!(!merchant.supports_currency(Currency::Eth));
    }

    #[test]
    fn test_merchant_with_crypto() {
        let merchant = Merchant::new("test", "Test Merchant")
            .with_crypto_address("0x1234567890123456789012345678901234567890");
        assert!(merchant.supports_currency(Currency::Eth));
        assert!(merchant.crypto_address.is_some());
    }

    #[test]
    fn test_payment_intent() {
        let merchant = Merchant::new("m1", "Test");
        let amount = PaymentAmount::from_dollars(10.0);
        let intent =
            PaymentIntent::new("agent-1", amount, merchant).with_description("Test payment");

        assert!(!intent.id.is_nil());
        assert_eq!(intent.status, PaymentStatus::Pending);
        assert!(intent.description.is_some());
    }

    #[test]
    fn test_payment_status_terminal() {
        assert!(!PaymentStatus::Pending.is_terminal());
        assert!(!PaymentStatus::Processing.is_terminal());
        assert!(PaymentStatus::Completed.is_terminal());
        assert!(PaymentStatus::Failed.is_terminal());
    }

    #[test]
    fn test_payment_result_success() {
        let result = PaymentResult::success(Uuid::new_v4());
        assert_eq!(result.status, PaymentStatus::Completed);
        assert!(result.completed_at.is_some());
    }

    #[test]
    fn test_payment_result_failed() {
        let result = PaymentResult::failed(Uuid::new_v4(), "Insufficient funds");
        assert_eq!(result.status, PaymentStatus::Failed);
        assert!(result.error.is_some());
    }
}
