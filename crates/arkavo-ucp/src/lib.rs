#![allow(clippy::significant_drop_tightening)]

//! Universal Commerce Protocol (UCP) implementation
//!
//! This crate provides:
//! - Payment intent creation and lifecycle management
//! - Commerce policy engine with budget integration
//! - Payment handlers for fiat (via budget) and crypto (EVM)
//! - MCP tools for AI agent payment operations
//!
//! All blockchain operations are offline (signing only) - no RPC calls are made.

pub mod error;
pub mod handler;
pub mod handlers;
pub mod policy;
pub mod tools;
pub mod tracker;
pub mod types;

pub use error::{Result, UcpError};
pub use handler::PaymentHandler;
pub use handlers::evm::chains;
pub use handlers::{BudgetPaymentHandler, EvmPaymentHandler};
pub use policy::{CommerceLimits, CommercePolicy, PolicyDecision};
pub use tools::{
    CreatePaymentIntentTool, ExecutePaymentTool, GetPaymentStatusTool, ListPaymentsTool, UcpState,
    register_tools,
};
pub use tracker::{PaymentRecord, PaymentStats, PaymentTracker};
pub use types::{Currency, Merchant, PaymentAmount, PaymentIntent, PaymentResult, PaymentStatus};

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_budget::{BudgetConfig, BudgetTracker};
    use arkavo_wallet::EvmKeypair;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_full_usd_payment_flow() {
        let config = BudgetConfig::default();
        let budget_tracker = Arc::new(BudgetTracker::new(config).await.unwrap());

        let tracker = PaymentTracker::new();
        let policy = CommercePolicy::new(budget_tracker.clone(), CommerceLimits::default());
        let handler = BudgetPaymentHandler::new(budget_tracker);

        let merchant = Merchant::new("store-1", "Test Store");
        let amount = PaymentAmount::from_dollars(2.50);
        let intent = PaymentIntent::new("agent-1", amount, merchant);

        let decision = policy.evaluate(&intent).await;
        assert!(decision.allowed);

        let id = tracker.create(intent.clone()).await;
        let result = handler.execute(&intent).await.unwrap();
        tracker.complete(id, result.clone()).await.unwrap();

        let record = tracker.get(id).await.unwrap();
        assert_eq!(record.status(), PaymentStatus::Completed);
    }

    #[tokio::test]
    async fn test_full_eth_payment_flow() {
        let keypair = Arc::new(EvmKeypair::generate());
        let handler = EvmPaymentHandler::new(keypair.clone(), 1);

        let merchant = Merchant::new("crypto-store", "Crypto Store")
            .with_crypto_address("0xdead000000000000000000000000000000000000");
        let amount = PaymentAmount::from_ether(0.01);
        let intent = PaymentIntent::new("agent-1", amount, merchant);

        let result = handler.execute(&intent).await.unwrap();
        assert_eq!(result.status, PaymentStatus::Signed);
        assert!(result.signed_tx.is_some());

        let tx_bytes = result.signed_tx.unwrap();
        assert!(!tx_bytes.is_empty());
    }

    #[tokio::test]
    async fn test_policy_blocks_large_payment() {
        let config = BudgetConfig::default();
        let budget_tracker = Arc::new(BudgetTracker::new(config).await.unwrap());

        let limits = CommerceLimits::default().with_max_single_payment(5000);
        let policy = CommercePolicy::new(budget_tracker, limits);

        let merchant = Merchant::new("store", "Store");
        let amount = PaymentAmount::from_dollars(100.0);
        let intent = PaymentIntent::new("agent-1", amount, merchant);

        let decision = policy.evaluate(&intent).await;
        assert!(!decision.allowed);
        assert!(decision.reason.unwrap().contains("exceeds"));
    }

    #[tokio::test]
    async fn test_policy_requires_confirmation() {
        let config = BudgetConfig::default();
        let budget_tracker = Arc::new(BudgetTracker::new(config).await.unwrap());

        let limits = CommerceLimits::default().with_confirmation_threshold(100);
        let policy = CommercePolicy::new(budget_tracker, limits);

        let merchant = Merchant::new("store", "Store");
        let amount = PaymentAmount::from_dollars(2.0);
        let intent = PaymentIntent::new("agent-1", amount, merchant);

        let decision = policy.evaluate(&intent).await;
        assert!(decision.allowed);
        assert!(decision.requires_confirmation);
    }

    #[tokio::test]
    async fn test_tracker_stats() {
        let tracker = PaymentTracker::new();

        let merchant = Merchant::new("store", "Store");

        for i in 0..3 {
            let amount = PaymentAmount::from_dollars(1.0 * (i + 1) as f64);
            let intent = PaymentIntent::new("agent-1", amount, merchant.clone());
            let id = tracker.create(intent).await;

            if i < 2 {
                tracker
                    .complete(id, PaymentResult::success(id))
                    .await
                    .unwrap();
            }
        }

        let stats = tracker.get_stats("agent-1").await;
        assert_eq!(stats.completed, 2);
        assert_eq!(stats.pending, 1);
    }

    #[test]
    fn test_currency_properties() {
        assert!(Currency::Usd.is_fiat());
        assert!(!Currency::Usd.is_native());

        assert!(!Currency::Eth.is_fiat());
        assert!(Currency::Eth.is_native());

        assert!(Currency::Usdc.is_erc20());
        assert_eq!(Currency::Usdc.decimals(), 6);
    }

    #[test]
    fn test_payment_amount_display() {
        let usd = PaymentAmount::from_dollars(99.99);
        assert_eq!(format!("{usd}"), "99.99 USD");

        let eth = PaymentAmount::from_ether(1.5);
        assert!(format!("{eth}").contains("ETH"));
    }
}
