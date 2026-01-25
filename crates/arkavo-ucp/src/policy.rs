use crate::error::{Result, UcpError};
use crate::types::{Currency, PaymentIntent};
use arkavo_budget::cost::TokenUsage;
use arkavo_budget::{BudgetTracker, TokenCost};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

/// Commerce policy limits
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommerceLimits {
    /// Maximum amount for a single payment (in cents)
    pub max_single_payment: u64,
    /// Maximum daily spending (in cents)
    pub daily_limit: u64,
    /// Amount above which confirmation is required (in cents)
    pub require_confirmation_above: u64,
    /// Blocked merchant IDs
    pub blocked_merchants: HashSet<String>,
    /// Allowed currencies
    pub allowed_currencies: HashSet<Currency>,
}

impl Default for CommerceLimits {
    fn default() -> Self {
        let mut allowed = HashSet::new();
        allowed.insert(Currency::Usd);
        allowed.insert(Currency::Eth);
        allowed.insert(Currency::Usdc);

        Self {
            max_single_payment: 10000,
            daily_limit: 50000,
            require_confirmation_above: 5000,
            blocked_merchants: HashSet::new(),
            allowed_currencies: allowed,
        }
    }
}

impl CommerceLimits {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_max_single_payment(mut self, cents: u64) -> Self {
        self.max_single_payment = cents;
        self
    }

    pub fn with_daily_limit(mut self, cents: u64) -> Self {
        self.daily_limit = cents;
        self
    }

    pub fn with_confirmation_threshold(mut self, cents: u64) -> Self {
        self.require_confirmation_above = cents;
        self
    }

    pub fn block_merchant(mut self, merchant_id: impl Into<String>) -> Self {
        self.blocked_merchants.insert(merchant_id.into());
        self
    }
}

/// Policy decision returned after evaluating a payment intent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub allowed: bool,
    pub requires_confirmation: bool,
    pub reason: Option<String>,
}

impl PolicyDecision {
    pub fn allow() -> Self {
        Self {
            allowed: true,
            requires_confirmation: false,
            reason: None,
        }
    }

    pub fn allow_with_confirmation(reason: impl Into<String>) -> Self {
        Self {
            allowed: true,
            requires_confirmation: true,
            reason: Some(reason.into()),
        }
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            requires_confirmation: false,
            reason: Some(reason.into()),
        }
    }
}

/// Commerce policy engine integrating with budget tracker
pub struct CommercePolicy {
    budget_tracker: Arc<BudgetTracker>,
    limits: CommerceLimits,
}

impl CommercePolicy {
    pub fn new(budget_tracker: Arc<BudgetTracker>, limits: CommerceLimits) -> Self {
        Self {
            budget_tracker,
            limits,
        }
    }

    /// Check if payment is within budget limits
    pub async fn can_afford(&self, intent: &PaymentIntent) -> bool {
        if intent.amount.currency != Currency::Usd {
            return true;
        }

        let cost = TokenCost::from_cents(intent.amount.value);
        self.budget_tracker
            .can_afford(&intent.agent_id, cost)
            .await
            .unwrap_or(false)
    }

    /// Evaluate full policy for a payment intent
    pub async fn evaluate(&self, intent: &PaymentIntent) -> PolicyDecision {
        if intent.is_expired() {
            return PolicyDecision::deny("Payment intent has expired");
        }

        if self.limits.blocked_merchants.contains(&intent.merchant.id) {
            return PolicyDecision::deny("Merchant is blocked by policy");
        }

        if !self.limits.allowed_currencies.contains(&intent.amount.currency) {
            return PolicyDecision::deny(format!(
                "Currency {} is not allowed",
                intent.amount.currency
            ));
        }

        if !intent.merchant.supports_currency(intent.amount.currency) {
            return PolicyDecision::deny(format!(
                "Merchant does not support {}",
                intent.amount.currency
            ));
        }

        let amount_in_cents = self.convert_to_cents(intent);

        if amount_in_cents > self.limits.max_single_payment {
            return PolicyDecision::deny(format!(
                "Amount ${:.2} exceeds single payment limit of ${:.2}",
                amount_in_cents as f64 / 100.0,
                self.limits.max_single_payment as f64 / 100.0
            ));
        }

        if intent.amount.currency == Currency::Usd && !self.can_afford(intent).await {
            return PolicyDecision::deny("Insufficient budget");
        }

        if amount_in_cents > self.limits.require_confirmation_above {
            return PolicyDecision::allow_with_confirmation(format!(
                "Amount ${:.2} exceeds confirmation threshold of ${:.2}",
                amount_in_cents as f64 / 100.0,
                self.limits.require_confirmation_above as f64 / 100.0
            ));
        }

        PolicyDecision::allow()
    }

    fn convert_to_cents(&self, intent: &PaymentIntent) -> u64 {
        match intent.amount.currency {
            Currency::Usd => intent.amount.value,
            Currency::Eth => 0,
            Currency::Usdc | Currency::Usdt => intent.amount.value / 10000,
        }
    }

    /// Record spending after successful payment
    pub async fn record_spending(&self, intent: &PaymentIntent) -> Result<()> {
        if intent.amount.currency != Currency::Usd {
            return Ok(());
        }

        let cost = TokenCost::from_cents(intent.amount.value);
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

    pub fn limits(&self) -> &CommerceLimits {
        &self.limits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Merchant, PaymentAmount};
    use arkavo_budget::BudgetConfig;

    async fn create_test_policy() -> CommercePolicy {
        let config = BudgetConfig::default();
        let tracker = BudgetTracker::new(config).await.unwrap();
        CommercePolicy::new(Arc::new(tracker), CommerceLimits::default())
    }

    #[tokio::test]
    async fn test_policy_allow_small_payment() {
        let policy = create_test_policy().await;
        let merchant = Merchant::new("test", "Test");
        let amount = PaymentAmount::from_dollars(2.0);
        let intent = PaymentIntent::new("agent-1", amount, merchant);

        let decision = policy.evaluate(&intent).await;
        assert!(decision.allowed);
        assert!(!decision.requires_confirmation);
    }

    #[tokio::test]
    async fn test_policy_require_confirmation() {
        let limits = CommerceLimits::default().with_confirmation_threshold(100);
        let config = BudgetConfig::default();
        let tracker = BudgetTracker::new(config).await.unwrap();
        let policy = CommercePolicy::new(Arc::new(tracker), limits);

        let merchant = Merchant::new("test", "Test");
        let amount = PaymentAmount::from_dollars(2.0);
        let intent = PaymentIntent::new("agent-1", amount, merchant);

        let decision = policy.evaluate(&intent).await;
        assert!(decision.allowed);
        assert!(decision.requires_confirmation);
    }

    #[tokio::test]
    async fn test_policy_deny_over_limit() {
        let limits = CommerceLimits::default().with_max_single_payment(1000);
        let config = BudgetConfig::default();
        let tracker = BudgetTracker::new(config).await.unwrap();
        let policy = CommercePolicy::new(Arc::new(tracker), limits);

        let merchant = Merchant::new("test", "Test");
        let amount = PaymentAmount::from_dollars(15.0);
        let intent = PaymentIntent::new("agent-1", amount, merchant);

        let decision = policy.evaluate(&intent).await;
        assert!(!decision.allowed);
        assert!(decision.reason.unwrap().contains("exceeds"));
    }

    #[tokio::test]
    async fn test_policy_blocked_merchant() {
        let limits = CommerceLimits::default().block_merchant("blocked-merchant");
        let config = BudgetConfig::default();
        let tracker = BudgetTracker::new(config).await.unwrap();
        let policy = CommercePolicy::new(Arc::new(tracker), limits);

        let merchant = Merchant::new("blocked-merchant", "Blocked");
        let amount = PaymentAmount::from_dollars(10.0);
        let intent = PaymentIntent::new("agent-1", amount, merchant);

        let decision = policy.evaluate(&intent).await;
        assert!(!decision.allowed);
        assert!(decision.reason.unwrap().contains("blocked"));
    }

    #[tokio::test]
    async fn test_policy_unsupported_currency() {
        let mut limits = CommerceLimits::default();
        limits.allowed_currencies.remove(&Currency::Eth);

        let config = BudgetConfig::default();
        let tracker = BudgetTracker::new(config).await.unwrap();
        let policy = CommercePolicy::new(Arc::new(tracker), limits);

        let merchant = Merchant::new("test", "Test")
            .with_crypto_address("0x1234567890123456789012345678901234567890");
        let amount = PaymentAmount::from_ether(0.1);
        let intent = PaymentIntent::new("agent-1", amount, merchant);

        let decision = policy.evaluate(&intent).await;
        assert!(!decision.allowed);
        assert!(decision.reason.unwrap().contains("not allowed"));
    }

    #[test]
    fn test_commerce_limits_builder() {
        let limits = CommerceLimits::new()
            .with_max_single_payment(5000)
            .with_daily_limit(20000)
            .with_confirmation_threshold(2500)
            .block_merchant("bad-merchant");

        assert_eq!(limits.max_single_payment, 5000);
        assert_eq!(limits.daily_limit, 20000);
        assert_eq!(limits.require_confirmation_above, 2500);
        assert!(limits.blocked_merchants.contains("bad-merchant"));
    }

    #[test]
    fn test_policy_decision_constructors() {
        let allow = PolicyDecision::allow();
        assert!(allow.allowed);
        assert!(!allow.requires_confirmation);

        let confirm = PolicyDecision::allow_with_confirmation("Large amount");
        assert!(confirm.allowed);
        assert!(confirm.requires_confirmation);

        let deny = PolicyDecision::deny("Not allowed");
        assert!(!deny.allowed);
        assert!(deny.reason.is_some());
    }
}
