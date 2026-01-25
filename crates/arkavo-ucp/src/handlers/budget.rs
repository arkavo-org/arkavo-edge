use crate::error::{Result, UcpError};
use crate::handler::PaymentHandler;
use crate::types::{Currency, PaymentIntent, PaymentResult};
use arkavo_budget::cost::TokenUsage;
use arkavo_budget::{BudgetTracker, TokenCost};
use async_trait::async_trait;
use std::sync::Arc;

/// Payment handler for fiat (USD) payments tracked via the budget system
pub struct BudgetPaymentHandler {
    budget_tracker: Arc<BudgetTracker>,
}

impl BudgetPaymentHandler {
    pub fn new(budget_tracker: Arc<BudgetTracker>) -> Self {
        Self { budget_tracker }
    }

    async fn check_and_record(&self, intent: &PaymentIntent) -> Result<()> {
        let cost = TokenCost::from_cents(intent.amount.value);

        let can_afford = self
            .budget_tracker
            .can_afford(&intent.agent_id, cost)
            .await
            .map_err(|e| UcpError::BudgetError(e.to_string()))?;

        if !can_afford {
            return Err(UcpError::PolicyViolation(
                "Insufficient budget for payment".to_string(),
            ));
        }

        self.budget_tracker
            .record_spending(
                intent.agent_id.clone(),
                "ucp".to_string(),
                "payment".to_string(),
                TokenUsage::new(0, 0),
                cost,
            )
            .await
            .map_err(|e| UcpError::BudgetError(e.to_string()))?;

        Ok(())
    }
}

#[async_trait]
impl PaymentHandler for BudgetPaymentHandler {
    fn name(&self) -> &'static str {
        "budget"
    }

    fn supported_currencies(&self) -> Vec<Currency> {
        vec![Currency::Usd]
    }

    async fn execute(&self, intent: &PaymentIntent) -> Result<PaymentResult> {
        if intent.amount.currency != Currency::Usd {
            return Err(UcpError::UnsupportedCurrency(format!(
                "Budget handler only supports USD, got {}",
                intent.amount.currency
            )));
        }

        match self.check_and_record(intent).await {
            Ok(()) => Ok(PaymentResult::success(intent.id)),
            Err(e) => Ok(PaymentResult::failed(intent.id, e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Merchant, PaymentAmount, PaymentStatus};
    use arkavo_budget::BudgetConfig;

    async fn create_handler() -> BudgetPaymentHandler {
        let config = BudgetConfig::default();
        let tracker = BudgetTracker::new(config).await.unwrap();
        BudgetPaymentHandler::new(Arc::new(tracker))
    }

    #[tokio::test]
    async fn test_supported_currencies() {
        let handler = create_handler().await;
        let currencies = handler.supported_currencies();
        assert!(currencies.contains(&Currency::Usd));
        assert!(!currencies.contains(&Currency::Eth));
    }

    #[tokio::test]
    async fn test_execute_usd_payment() {
        let handler = create_handler().await;
        let merchant = Merchant::new("test", "Test");
        let amount = PaymentAmount::from_dollars(2.0);
        let intent = PaymentIntent::new("agent-1", amount, merchant);

        let result = handler.execute(&intent).await.unwrap();
        assert_eq!(result.status, PaymentStatus::Completed);
    }

    #[tokio::test]
    async fn test_reject_non_usd() {
        let handler = create_handler().await;
        let merchant = Merchant::new("test", "Test")
            .with_crypto_address("0x1234567890123456789012345678901234567890");
        let amount = PaymentAmount::from_ether(0.1);
        let intent = PaymentIntent::new("agent-1", amount, merchant);

        let result = handler.execute(&intent).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_can_handle() {
        let handler = create_handler().await;
        let merchant = Merchant::new("test", "Test");

        let usd_intent =
            PaymentIntent::new("agent", PaymentAmount::from_dollars(2.0), merchant.clone());
        assert!(handler.can_handle(&usd_intent));

        let eth_intent = PaymentIntent::new("agent", PaymentAmount::from_ether(0.1), merchant);
        assert!(!handler.can_handle(&eth_intent));
    }

    #[tokio::test]
    async fn test_handler_name() {
        let handler = create_handler().await;
        assert_eq!(handler.name(), "budget");
    }
}
