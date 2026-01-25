use crate::error::Result;
use crate::types::{Currency, PaymentIntent, PaymentResult};
use async_trait::async_trait;

/// Trait for payment handlers that can execute different payment types
#[async_trait]
pub trait PaymentHandler: Send + Sync {
    /// Handler name for identification
    fn name(&self) -> &'static str;

    /// Currencies supported by this handler
    fn supported_currencies(&self) -> Vec<Currency>;

    /// Execute a payment intent
    async fn execute(&self, intent: &PaymentIntent) -> Result<PaymentResult>;

    /// Check if handler can process this intent
    fn can_handle(&self, intent: &PaymentIntent) -> bool {
        self.supported_currencies()
            .contains(&intent.amount.currency)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Merchant, PaymentAmount, PaymentStatus};

    struct MockHandler {
        currencies: Vec<Currency>,
    }

    #[async_trait]
    impl PaymentHandler for MockHandler {
        fn name(&self) -> &'static str {
            "mock"
        }

        fn supported_currencies(&self) -> Vec<Currency> {
            self.currencies.clone()
        }

        async fn execute(&self, intent: &PaymentIntent) -> Result<PaymentResult> {
            Ok(PaymentResult::success(intent.id))
        }
    }

    #[test]
    fn test_can_handle() {
        let handler = MockHandler {
            currencies: vec![Currency::Usd, Currency::Eth],
        };

        let merchant = Merchant::new("test", "Test");

        let usd_intent =
            PaymentIntent::new("agent", PaymentAmount::from_dollars(10.0), merchant.clone());
        assert!(handler.can_handle(&usd_intent));

        let usdc_intent = PaymentIntent::new(
            "agent",
            PaymentAmount {
                value: 1000000,
                currency: Currency::Usdc,
            },
            merchant,
        );
        assert!(!handler.can_handle(&usdc_intent));
    }

    #[tokio::test]
    async fn test_execute() {
        let handler = MockHandler {
            currencies: vec![Currency::Usd],
        };

        let merchant = Merchant::new("test", "Test");
        let intent = PaymentIntent::new("agent", PaymentAmount::from_dollars(10.0), merchant);

        let result = handler.execute(&intent).await.unwrap();
        assert_eq!(result.status, PaymentStatus::Completed);
    }
}
