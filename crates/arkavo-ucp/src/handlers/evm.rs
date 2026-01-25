use crate::error::{Result, UcpError};
use crate::handler::PaymentHandler;
use crate::types::{Currency, PaymentIntent, PaymentResult};
use arkavo_wallet::{EvmKeypair, Transaction, TransactionBuilder};
use async_trait::async_trait;
use std::sync::Arc;

/// Payment handler for EVM-compatible blockchain transactions
///
/// This handler performs offline signing only - it does not broadcast transactions.
/// The signed transaction bytes are returned in the result for external broadcast.
pub struct EvmPaymentHandler {
    keypair: Arc<EvmKeypair>,
    chain_id: u64,
    gas_price_gwei: u64,
    gas_limit: u64,
}

impl EvmPaymentHandler {
    /// Create a new EVM payment handler
    pub fn new(keypair: Arc<EvmKeypair>, chain_id: u64) -> Self {
        Self {
            keypair,
            chain_id,
            gas_price_gwei: 20,
            gas_limit: 21000,
        }
    }

    /// Set the gas price in gwei
    pub fn with_gas_price(mut self, gwei: u64) -> Self {
        self.gas_price_gwei = gwei;
        self
    }

    /// Set the gas limit
    pub fn with_gas_limit(mut self, limit: u64) -> Self {
        self.gas_limit = limit;
        self
    }

    /// Get the wallet address
    pub fn address(&self) -> arkavo_wallet::EvmAddress {
        self.keypair.address()
    }

    fn build_transaction(&self, intent: &PaymentIntent, nonce: u64) -> Result<Transaction> {
        let to = intent.merchant.crypto_address()?;
        let value = intent.amount.to_wei()?;

        let tx = TransactionBuilder::default()
            .nonce(nonce)
            .gas_price_gwei(self.gas_price_gwei)
            .gas_limit(self.gas_limit)
            .to(to)
            .value(value)
            .chain_id(self.chain_id)
            .build();

        Ok(tx)
    }
}

#[async_trait]
impl PaymentHandler for EvmPaymentHandler {
    fn name(&self) -> &'static str {
        "evm"
    }

    fn supported_currencies(&self) -> Vec<Currency> {
        vec![Currency::Eth]
    }

    async fn execute(&self, intent: &PaymentIntent) -> Result<PaymentResult> {
        if !self.can_handle(intent) {
            return Err(UcpError::UnsupportedCurrency(format!(
                "EVM handler supports ETH, got {}",
                intent.amount.currency
            )));
        }

        // NOTE: Nonce defaults to 0 for offline signing.
        // For production use, nonce should be fetched from the blockchain
        // or tracked externally. This implementation is suitable for:
        // - Single-use wallets
        // - Testing and development
        // - Scenarios where nonce is managed by the caller via metadata
        let nonce = intent
            .metadata
            .as_ref()
            .and_then(|m| m.get("nonce"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let tx = self.build_transaction(intent, nonce)?;
        let signed = tx.sign(&self.keypair);
        let tx_bytes = signed.to_rlp_bytes();

        Ok(PaymentResult::signed(intent.id, tx_bytes))
    }
}

/// Common chain IDs
pub mod chains {
    /// Ethereum Mainnet
    pub const ETHEREUM: u64 = 1;
    /// Polygon Mainnet
    pub const POLYGON: u64 = 137;
    /// Arbitrum One
    pub const ARBITRUM: u64 = 42161;
    /// Optimism
    pub const OPTIMISM: u64 = 10;
    /// Base
    pub const BASE: u64 = 8453;
    /// Goerli Testnet
    pub const GOERLI: u64 = 5;
    /// Sepolia Testnet
    pub const SEPOLIA: u64 = 11_155_111;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Merchant, PaymentAmount, PaymentStatus};

    fn create_handler() -> EvmPaymentHandler {
        let keypair = Arc::new(EvmKeypair::generate());
        EvmPaymentHandler::new(keypair, chains::ETHEREUM)
    }

    fn create_intent_with_crypto(address: &str) -> PaymentIntent {
        let merchant = Merchant::new("test", "Test Merchant").with_crypto_address(address);
        let amount = PaymentAmount::from_ether(0.1);
        PaymentIntent::new("agent-1", amount, merchant)
    }

    #[test]
    fn test_handler_name() {
        let handler = create_handler();
        assert_eq!(handler.name(), "evm");
    }

    #[test]
    fn test_supported_currencies() {
        let handler = create_handler();
        let currencies = handler.supported_currencies();
        assert!(currencies.contains(&Currency::Eth));
        assert!(!currencies.contains(&Currency::Usd));
    }

    #[test]
    fn test_can_handle() {
        let handler = create_handler();
        let merchant = Merchant::new("test", "Test")
            .with_crypto_address("0xdead000000000000000000000000000000000000");

        let eth_intent =
            PaymentIntent::new("agent", PaymentAmount::from_ether(0.1), merchant.clone());
        assert!(handler.can_handle(&eth_intent));

        let usd_intent = PaymentIntent::new(
            "agent",
            PaymentAmount::from_dollars(10.0),
            Merchant::new("test", "Test"),
        );
        assert!(!handler.can_handle(&usd_intent));
    }

    #[tokio::test]
    async fn test_execute_eth_payment() {
        let handler = create_handler();
        let intent = create_intent_with_crypto("0xdead000000000000000000000000000000000000");

        let result = handler.execute(&intent).await.unwrap();
        assert_eq!(result.status, PaymentStatus::Signed);
        assert!(result.signed_tx.is_some());
    }

    #[tokio::test]
    async fn test_execute_returns_signed_tx() {
        let handler = create_handler();
        let intent = create_intent_with_crypto("0xdead000000000000000000000000000000000000");

        let result = handler.execute(&intent).await.unwrap();
        let tx_bytes = result.signed_tx.unwrap();
        assert!(!tx_bytes.is_empty());
    }

    #[tokio::test]
    async fn test_reject_non_eth() {
        let handler = create_handler();
        let merchant = Merchant::new("test", "Test");
        let amount = PaymentAmount::from_dollars(10.0);
        let intent = PaymentIntent::new("agent-1", amount, merchant);

        let result = handler.execute(&intent).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_with_gas_settings() {
        let keypair = Arc::new(EvmKeypair::generate());
        let handler = EvmPaymentHandler::new(keypair, chains::POLYGON)
            .with_gas_price(50)
            .with_gas_limit(100_000);

        assert_eq!(handler.gas_price_gwei, 50);
        assert_eq!(handler.gas_limit, 100_000);
    }

    #[test]
    fn test_get_address() {
        let handler = create_handler();
        let addr = handler.address();
        assert_eq!(addr.as_bytes().len(), 20);
    }

    #[test]
    fn test_chain_ids() {
        assert_eq!(chains::ETHEREUM, 1);
        assert_eq!(chains::POLYGON, 137);
        assert_eq!(chains::ARBITRUM, 42161);
    }
}
