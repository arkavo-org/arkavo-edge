#[cfg(test)]
mod tests {
    use crate::config::{BudgetConfig, BudgetLimits, BudgetThresholds};
    use crate::cost::TokenCost;
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
        assert!(tracker
            .can_afford("test-agent", TokenCost::from_dollars(5.0))
            .await
            .unwrap());

        // Should not be able to afford $15
        assert!(!tracker
            .can_afford("test-agent", TokenCost::from_dollars(15.0))
            .await
            .unwrap());
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
            if let crate::tracker::BudgetEvent::ThresholdExceeded(alert) = event {
                if alert.alert_type == crate::config::AlertType::Warning {
                    found_warning = true;
                    break;
                }
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
        assert!(tracker
            .can_afford("limited-agent", TokenCost::from_cents(25))
            .await
            .unwrap());

        // Check agent cannot afford beyond its limit
        assert!(!tracker
            .can_afford("limited-agent", TokenCost::from_dollars(1.0))
            .await
            .unwrap());

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
        assert!(!tracker
            .can_afford("agent1", TokenCost::from_cents(1))
            .await
            .unwrap());

        // Check for exhausted alert
        let mut found_exhausted = false;
        while let Ok(event) = event_rx.try_recv() {
            if let crate::tracker::BudgetEvent::BudgetExhausted(alert) = event {
                if alert.alert_type == crate::config::AlertType::Exhausted {
                    found_exhausted = true;
                    break;
                }
            }
        }
        assert!(found_exhausted, "Should have received exhausted alert");
    }

    #[tokio::test]
    async fn test_cost_estimation() {
        use crate::provider_costs::ProviderPricing;
        let pricing = ProviderPricing::new();

        // Test OpenAI GPT-3.5-turbo pricing
        let cost = pricing
            .estimate_cost("openai", "gpt-3.5-turbo", 1000, 500)
            .expect("Should have pricing for GPT-3.5-turbo");

        // 1000 tokens input @ $0.50/1k + 500 tokens output @ $1.50/1k = $0.50 + $0.75 = $1.25
        assert_eq!(cost.as_cents(), 125);

        // Test free models
        let cost = pricing
            .estimate_cost("ollama", "llama3.2:latest", 10000, 5000)
            .expect("Should have pricing for Llama");

        assert_eq!(cost, TokenCost::ZERO);
    }
}
