//! Carry sub-cent request charges forward instead of silently treating them as free.
use crate::{BudgetTracker, TokenCost, cost::TokenUsage, tracker::SpendingRecord};

impl BudgetTracker {
    /// Record a provider's cost estimate in dollars. The public ledger remains in
    /// whole cents; fractions accumulate independently for each attributed model.
    pub async fn record_spending_precise(
        &self,
        agent_id: String,
        provider: String,
        model: String,
        usage: TokenUsage,
        dollars: f64,
    ) -> anyhow::Result<SpendingRecord> {
        anyhow::ensure!(
            dollars.is_finite() && dollars >= 0.0,
            "Invalid provider cost"
        );
        let key = (agent_id.clone(), provider.clone(), model.clone());
        // Hold through recording so concurrent calls cannot spend the same carry.
        let mut fractions = self.fractional_spending.lock().await;
        let micros = (dollars * 1_000_000.0).round() as u64;
        let combined = micros.saturating_add(*fractions.get(&key).unwrap_or(&0));
        let record = self
            .record_spending(
                agent_id,
                provider,
                model,
                usage,
                TokenCost::from_cents(combined / 10_000),
            )
            .await?;
        fractions.insert(key, combined % 10_000);
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BudgetConfig;
    use arkavo_test_macros::spec;

    #[spec("ASTRA-005")]
    #[tokio::test]
    async fn small_requests_accumulate_without_rounding_each_to_a_cent() {
        let tracker = BudgetTracker::new(BudgetConfig::default()).await.unwrap();
        for _ in 0..25 {
            tracker
                .record_spending_precise(
                    "worker".into(),
                    "openai".into(),
                    "gpt-6-astra".into(),
                    TokenUsage::new(1, 0),
                    0.0004,
                )
                .await
                .unwrap();
        }
        assert_eq!(tracker.get_status().await.total_spent.as_cents(), 1);
        let history = tracker.get_spending_history(30).await;
        assert_eq!(history.len(), 25);
        assert_eq!(
            history.iter().map(|r| r.usage.total_tokens()).sum::<u32>(),
            25
        );
        assert_eq!(history.iter().map(|r| r.cost.as_cents()).sum::<u64>(), 1);
    }

    #[spec("ASTRA-005")]
    #[tokio::test]
    async fn fractions_keep_model_attribution() {
        let tracker = BudgetTracker::new(BudgetConfig::default()).await.unwrap();
        for model in ["first", "second", "first"] {
            tracker
                .record_spending_precise(
                    "worker".into(),
                    "openai".into(),
                    model.into(),
                    TokenUsage::new(1, 0),
                    0.006,
                )
                .await
                .unwrap();
        }
        let history = tracker.get_spending_history(3).await;
        assert_eq!(
            history
                .iter()
                .filter(|r| r.model == "first")
                .map(|r| r.cost.as_cents())
                .sum::<u64>(),
            1
        );
        assert_eq!(
            history
                .iter()
                .filter(|r| r.model == "second")
                .map(|r| r.cost.as_cents())
                .sum::<u64>(),
            0
        );
    }
    #[spec("ASTRA-005")]
    #[tokio::test]
    async fn concurrent_fractional_charges_are_not_lost() {
        let tracker =
            std::sync::Arc::new(BudgetTracker::new(BudgetConfig::default()).await.unwrap());
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..100 {
            let tracker = tracker.clone();
            tasks.spawn(async move {
                tracker
                    .record_spending_precise(
                        "worker".into(),
                        "openai".into(),
                        "gpt-6-astra".into(),
                        TokenUsage::new(1, 0),
                        0.0004,
                    )
                    .await
                    .unwrap();
            });
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
        assert_eq!(tracker.get_status().await.total_spent.as_cents(), 4);
        assert_eq!(tracker.get_spending_history(100).await.len(), 100);
    }
}
