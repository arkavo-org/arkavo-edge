#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use crate::config::{BudgetConfig, BudgetLimits, BudgetThresholds};
    use crate::cost::{TokenCost, TokenUsage};
    use crate::tracker::BudgetTracker;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_budget_tracker_creation() {
        let config = BudgetConfig::default();
        let tracker = BudgetTracker::new(config).await.unwrap();

        let status = tracker.get_status().await;
        assert_eq!(status.session_spent, TokenCost::ZERO);
        assert!(status.session_limit.is_some());
    }

    #[tokio::test]
    async fn test_can_afford_within_budget() {
        let config = BudgetConfig {
            limits: BudgetLimits {
                session_limit: Some(TokenCost::from_dollars(10.0)),
                ..Default::default()
            },
            ..Default::default()
        };

        let tracker = BudgetTracker::new(config).await.unwrap();

        // Should be able to afford $5
        assert!(
            tracker
                .can_afford("test-agent", TokenCost::from_dollars(5.0))
                .await
                .unwrap()
        );

        // Should not be able to afford $15
        assert!(
            !tracker
                .can_afford("test-agent", TokenCost::from_dollars(15.0))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn try_spend_caps_concurrent_reservations_at_budget() {
        // The cost gate relies on try_spend holding the budget lock across
        // check-and-deduct: when many agents reserve concurrently against a
        // budget that affords only a few, the deductions serialize so the
        // total can never exceed the limit. A non-atomic check-then-deduct
        // (the old can_afford gate) would let every concurrent caller observe
        // the same remaining budget and all pass — the overspend race.
        let config = BudgetConfig {
            limits: BudgetLimits {
                session_limit: Some(TokenCost::from_dollars(3.0)),
                ..Default::default()
            },
            ..Default::default()
        };
        let tracker = Arc::new(BudgetTracker::new(config).await.unwrap());

        // 10 concurrent $1 reservations against a $3 session limit.
        let mut handles = Vec::new();
        for i in 0..10 {
            let tracker = Arc::clone(&tracker);
            handles.push(tokio::spawn(async move {
                tracker
                    .try_spend(
                        format!("agent-{i}"),
                        "test".to_string(),
                        "model".to_string(),
                        TokenUsage::new(0, 0),
                        TokenCost::from_dollars(1.0),
                    )
                    .await
                    .is_ok()
            }));
        }

        let mut succeeded = 0;
        for h in handles {
            if h.await.unwrap() {
                succeeded += 1;
            }
        }

        assert_eq!(
            succeeded, 3,
            "exactly 3 of 10 concurrent $1 reservations may succeed against a \
             $3 limit; got {succeeded} — the atomic check-and-deduct leaked budget"
        );
        let status = tracker.get_status().await;
        assert!(
            status.session_spent <= TokenCost::from_dollars(3.0),
            "total reserved must never exceed the session limit, got {}",
            status.session_spent
        );
    }

    #[tokio::test]
    async fn test_spending_tracking() {
        let config = BudgetConfig {
            limits: BudgetLimits {
                session_limit: Some(TokenCost::from_dollars(10.0)),
                ..Default::default()
            },
            ..Default::default()
        };

        let tracker = Arc::new(BudgetTracker::new(config).await.unwrap());

        // Record some spending
        use crate::cost::TokenUsage;
        let usage = TokenUsage::new(500, 250);
        tracker
            .record_spending(
                "agent1".to_string(),
                "openai".to_string(),
                "gpt-4".to_string(),
                usage,
                TokenCost::from_dollars(2.0),
            )
            .await
            .unwrap();

        let status = tracker.get_status().await;
        assert_eq!(status.session_spent, TokenCost::from_dollars(2.0));
        assert_eq!(status.session_remaining, Some(TokenCost::from_dollars(8.0)));
        assert_eq!(status.session_usage_percent, 20.0);
    }

    #[tokio::test]
    async fn test_budget_alerts() {
        let config = BudgetConfig {
            limits: BudgetLimits {
                session_limit: Some(TokenCost::from_dollars(1.0)),
                ..Default::default()
            },
            thresholds: BudgetThresholds {
                warning_percent: 50,
                critical_percent: 80,
                emergency_percent: 95,
            },
            ..Default::default()
        };

        let tracker = Arc::new(BudgetTracker::new(config).await.unwrap());
        let mut event_rx = tracker.subscribe_events();

        // Spend 60% - should trigger warning
        use crate::cost::TokenUsage;
        let usage = TokenUsage::new(100, 50);
        tracker
            .record_spending(
                "agent1".to_string(),
                "openai".to_string(),
                "gpt-4".to_string(),
                usage,
                TokenCost::from_cents(60),
            )
            .await
            .unwrap();

        // Check for warning alert
        let mut found_warning = false;
        while let Ok(event) = event_rx.try_recv() {
            if let crate::tracker::BudgetEvent::ThresholdExceeded(alert) = event
                && alert.alert_type == crate::config::AlertType::Warning
            {
                found_warning = true;
                break;
            }
        }
        assert!(
            found_warning,
            "Should have received warning alert at 60% usage"
        );
    }

    #[tokio::test]
    async fn test_agent_specific_budgets() {
        // Create config with agent-specific budget
        use crate::config::AgentBudget;
        let agent_budget = AgentBudget::new("limited-agent".to_string())
            .with_session_limit(TokenCost::from_dollars(0.5));

        let mut config = BudgetConfig::default();
        config
            .agent_budgets
            .insert("limited-agent".to_string(), agent_budget);
        let tracker = Arc::new(BudgetTracker::new(config).await.unwrap());

        // Check agent can afford within its limit
        assert!(
            tracker
                .can_afford("limited-agent", TokenCost::from_cents(25))
                .await
                .unwrap()
        );

        // Check agent cannot afford beyond its limit
        assert!(
            !tracker
                .can_afford("limited-agent", TokenCost::from_dollars(1.0))
                .await
                .unwrap()
        );

        // Record spending for the agent
        use crate::cost::TokenUsage;
        let usage = TokenUsage::new(100, 50);
        tracker
            .record_spending(
                "limited-agent".to_string(),
                "openai".to_string(),
                "gpt-3.5-turbo".to_string(),
                usage,
                TokenCost::from_cents(30),
            )
            .await
            .unwrap();

        // Check agent status
        let status = tracker.get_agent_status("limited-agent").await.unwrap();
        assert_eq!(status.session_spent, TokenCost::from_cents(30));
    }

    #[tokio::test]
    async fn test_spending_history() {
        let config = BudgetConfig::default();
        let tracker = Arc::new(BudgetTracker::new(config).await.unwrap());

        // Record multiple spending entries
        use crate::cost::TokenUsage;
        let usage1 = TokenUsage::new(200, 100);
        tracker
            .record_spending(
                "agent1".to_string(),
                "openai".to_string(),
                "gpt-4".to_string(),
                usage1,
                TokenCost::from_dollars(1.0),
            )
            .await
            .unwrap();

        let usage2 = TokenUsage::new(100, 50);
        tracker
            .record_spending(
                "agent2".to_string(),
                "anthropic".to_string(),
                "claude-3-haiku".to_string(),
                usage2,
                TokenCost::from_cents(50),
            )
            .await
            .unwrap();

        // Get spending history
        let history = tracker.get_spending_history(10).await;
        assert_eq!(history.len(), 2);

        // Get filtered history by checking agent_id
        let agent1_history: Vec<_> = history
            .into_iter()
            .filter(|r| r.agent_id == "agent1")
            .collect();
        assert_eq!(agent1_history.len(), 1);
    }

    #[tokio::test]
    async fn test_budget_exhaustion() {
        let config = BudgetConfig {
            limits: BudgetLimits {
                session_limit: Some(TokenCost::from_cents(100)),
                ..Default::default()
            },
            ..Default::default()
        };

        let tracker = Arc::new(BudgetTracker::new(config).await.unwrap());
        let mut event_rx = tracker.subscribe_events();

        // Spend the entire budget
        use crate::cost::TokenUsage;
        let usage = TokenUsage::new(300, 200);
        tracker
            .record_spending(
                "agent1".to_string(),
                "openai".to_string(),
                "gpt-4".to_string(),
                usage,
                TokenCost::from_cents(100),
            )
            .await
            .unwrap();

        // Should not be able to afford anything more
        assert!(
            !tracker
                .can_afford("agent1", TokenCost::from_cents(1))
                .await
                .unwrap()
        );

        // Check for exhausted alert
        let mut found_exhausted = false;
        while let Ok(event) = event_rx.try_recv() {
            if let crate::tracker::BudgetEvent::BudgetExhausted(alert) = event
                && alert.alert_type == crate::config::AlertType::Exhausted
            {
                found_exhausted = true;
                break;
            }
        }
        assert!(found_exhausted, "Should have received exhausted alert");
    }

    #[tokio::test]
    async fn test_cost_estimation() {
        use crate::provider_costs::{PricingEntry, ProviderPricing};
        let mut pricing = ProviderPricing::new();

        // Runtime-loaded pricing (no hardcoded defaults)
        pricing.load_from_entries(&[
            PricingEntry {
                model_id: "gpt-3.5-turbo".into(),
                provider: "openai".into(),
                input_cents_per_mtok: 50,
                output_cents_per_mtok: 150,
                cached_input_cents_per_mtok: None,
                cache_write_cents_per_mtok: None,
                context_window: Some(16385),
                max_output_tokens: Some(4096),
            },
            PricingEntry {
                model_id: "llama3.2:latest".into(),
                provider: "ollama".into(),
                input_cents_per_mtok: 0,
                output_cents_per_mtok: 0,
                cached_input_cents_per_mtok: None,
                cache_write_cents_per_mtok: None,
                context_window: Some(8192),
                max_output_tokens: Some(4096),
            },
        ]);

        // Test OpenAI GPT-3.5-turbo pricing (rates are per-MTok now).
        let cost = pricing
            .estimate_cost("openai", "gpt-3.5-turbo", 1_000_000, 500_000)
            .expect("Should have pricing for GPT-3.5-turbo");

        // 1M * 50/1M + 0.5M * 150/1M = 50 + 75 = 125
        assert_eq!(cost.as_cents(), 125);

        // Test free models
        let cost = pricing
            .estimate_cost("ollama", "llama3.2:latest", 10000, 5000)
            .expect("Should have pricing for Llama");

        assert_eq!(cost, TokenCost::ZERO);
    }
}
