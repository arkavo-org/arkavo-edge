#![allow(clippy::disallowed_methods)]
#![allow(clippy::future_not_send)]
#![allow(dead_code)]
#![allow(clippy::format_push_string)]
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::unnecessary_debug_formatting)]
#![allow(clippy::lines_filter_map_ok)]
#![allow(clippy::manual_strip)]
#![allow(clippy::needless_continue)]
#![allow(unused_imports)]
#![allow(clippy::zombie_processes)]
#![allow(clippy::unreadable_literal)]
#![allow(clippy::ignore_without_reason)]
#![allow(clippy::unnecessary_unwrap)]
#![allow(unreachable_pub)]

mod e2e_test_infrastructure;

use arkavo_budget::{
    BudgetConfig, BudgetLimits, BudgetManager, BudgetThresholds, BudgetTracker, TokenCost,
};
use arkavo_kimi::provider::{Message, Role};
use arkavo_kimi::{KimiClient, KimiConfig, Model};
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires MOONSHOT_API_KEY"]
async fn test_kimi_budget_tracking() -> Result<(), Box<dyn std::error::Error>> {
    // Skip if API key not set
    let api_key = match std::env::var("MOONSHOT_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            eprintln!("MOONSHOT_API_KEY not set, skipping budget tracking test");
            return Ok(());
        }
    };

    // Create a budget manager with a small limit for testing
    let budget_config = BudgetConfig {
        limits: BudgetLimits {
            session_limit: Some(TokenCost::from_dollars(1.0)), // $1 limit for testing
            hourly_limit: Some(TokenCost::from_dollars(0.5)),
            daily_limit: Some(TokenCost::from_dollars(2.0)),
            monthly_limit: Some(TokenCost::from_dollars(10.0)),
            total_limit: None,
        },
        thresholds: BudgetThresholds {
            warning_percent: 50,
            critical_percent: 80,
            emergency_percent: 95,
        },
        ..Default::default()
    };

    let budget_manager = Arc::new(BudgetManager::new(budget_config).await?);
    let tracker = budget_manager.tracker();

    // Subscribe to budget events
    let mut event_rx = tracker.subscribe_events();

    // Create Kimi client
    let kimi_config = KimiConfig {
        api_key,
        model: Model::MoonshotV1_32k, // Use cheaper 32k model
        ..Default::default()
    };

    let client = KimiClient::new(kimi_config)?;

    // Test 1: Make a simple request and verify budget tracking
    println!("Test 1: Simple request with budget tracking");
    let messages = vec![Message {
        role: Role::User,
        content: "Say 'Hello, budget tracking!' in exactly 5 words.".to_string(),
        images: None,
    }];

    // Estimate costs before the request
    let estimated_input_tokens = 20u32; // Rough estimate
    let estimated_output_tokens = 10u32;
    use arkavo_budget::provider_costs::ProviderPricing;
    let pricing = ProviderPricing::new();
    let estimated_cost = pricing
        .estimate_cost(
            "kimi",
            "moonshot-v1-32k",
            estimated_input_tokens,
            estimated_output_tokens,
        )
        .expect("Should be able to estimate cost");

    println!("Estimated cost: ${:.4}", estimated_cost.as_dollars());

    // Check if we can afford it
    let can_afford = tracker.can_afford("test-agent-1", estimated_cost).await?;
    assert!(can_afford, "Should be able to afford initial request");

    // Make the actual request
    let response = client
        .create_chat_completion(messages.clone(), None, None, None, None)
        .await?;

    println!("Response: {response}");

    // In a real implementation, we would get actual token counts from the response
    // For now, we'll simulate spending based on estimates
    let actual_input_tokens = messages[0].content.len() as u32 / 4; // Rough token estimate
    let actual_output_tokens = response.len() as u32 / 4;
    let actual_cost = pricing
        .estimate_cost(
            "kimi",
            "moonshot-v1-32k",
            actual_input_tokens,
            actual_output_tokens,
        )
        .expect("Should be able to calculate actual cost");

    // Record the spending
    use arkavo_budget::cost::TokenUsage;
    let usage = TokenUsage::new(actual_input_tokens, actual_output_tokens);
    tracker
        .record_spending(
            "test-agent-1".to_string(),
            "kimi".to_string(),
            "moonshot-v1-32k".to_string(),
            usage,
            actual_cost,
        )
        .await?;

    // Check budget status
    let status = tracker.get_status().await;
    println!("Budget status after first request:");
    println!("  Session spent: ${:.4}", status.session_spent.as_dollars());
    println!(
        "  Session remaining: ${:.4}",
        status
            .session_remaining
            .map(|c| c.as_dollars())
            .unwrap_or(0.0)
    );
    println!("  Session usage: {:.1}%", status.session_usage_percent);

    assert!(
        status.session_spent > TokenCost::ZERO,
        "Should have recorded spending"
    );

    // Test 2: Multiple requests to trigger budget alerts
    println!("\nTest 2: Multiple requests to trigger alerts");
    let mut total_spent = status.session_spent;
    let mut request_count = 1;

    while total_spent.as_dollars() < 0.5 && request_count < 20 {
        request_count += 1;

        let messages = vec![Message {
            role: Role::User,
            content: format!("Count to {request_count}. Be very brief."),
            images: None,
        }];

        // Check if we can afford another request
        let estimated_cost = pricing
            .estimate_cost("kimi", "moonshot-v1-32k", 15, 20)
            .expect("Should estimate cost");

        if !tracker.can_afford("test-agent-1", estimated_cost).await? {
            println!("Cannot afford request #{request_count}, stopping");
            break;
        }

        // Make request
        let response = client
            .create_chat_completion(messages, None, None, None, None)
            .await?;

        // Record spending
        let cost = pricing
            .estimate_cost("kimi", "moonshot-v1-32k", 15, response.len() as u32 / 4)
            .expect("Should calculate cost");

        let usage = TokenUsage::new(15, response.len() as u32 / 4);
        tracker
            .record_spending(
                "test-agent-1".to_string(),
                "kimi".to_string(),
                "moonshot-v1-32k".to_string(),
                usage,
                cost,
            )
            .await?;

        total_spent += cost;
        println!(
            "Request #{}: spent ${:.4} (total: ${:.4})",
            request_count,
            cost.as_dollars(),
            total_spent.as_dollars()
        );
    }

    // Check for budget events
    println!("\nChecking for budget events...");
    let mut alert_count = 0;
    while let Ok(event) = event_rx.try_recv() {
        use arkavo_budget::tracker::BudgetEvent;
        match event {
            BudgetEvent::ThresholdExceeded(alert) | BudgetEvent::BudgetExhausted(alert) => {
                println!("Received alert: {:?} - {}", alert.alert_type, alert.message);
                alert_count += 1;
            }
            BudgetEvent::SpendingRecorded(record) => {
                println!(
                    "Spending recorded: ${:.4} for {}",
                    record.cost.as_dollars(),
                    record.model
                );
            }
            _ => {}
        }
    }

    assert!(
        alert_count > 0,
        "Should have received at least one budget alert"
    );

    // Test 3: Verify budget enforcement
    println!("\nTest 3: Budget enforcement");
    let final_status = tracker.get_status().await;
    println!("Final budget status:");
    println!(
        "  Session spent: ${:.4}",
        final_status.session_spent.as_dollars()
    );
    println!(
        "  Session usage: {:.1}%",
        final_status.session_usage_percent
    );

    // Try to make a request that would exceed the budget
    let expensive_cost = TokenCost::from_dollars(2.0);
    let can_afford_expensive = tracker.can_afford("test-agent-1", expensive_cost).await?;
    assert!(
        !can_afford_expensive,
        "Should not be able to afford expensive request"
    );

    // Test 4: Agent-specific budgets
    println!("\nTest 4: Agent-specific budgets");
    use arkavo_budget::config::AgentBudget;

    let agent_budget =
        AgentBudget::new("limited-agent".to_string()).with_session_limit(TokenCost::from_cents(10)); // 10 cents limit

    // Create a new tracker with agent-specific budget
    let mut config = BudgetConfig::default();
    config
        .agent_budgets
        .insert("limited-agent".to_string(), agent_budget);
    let tracker_with_agent = Arc::new(BudgetTracker::new(config).await?);

    // Check that the limited agent can afford within its budget
    let can_afford = tracker_with_agent
        .can_afford("limited-agent", TokenCost::from_cents(5))
        .await?;
    assert!(
        can_afford,
        "Agent should be able to afford within its limit"
    );

    let cannot_afford = tracker_with_agent
        .can_afford("limited-agent", TokenCost::from_cents(15))
        .await?;
    assert!(
        !cannot_afford,
        "Agent should not be able to afford beyond its limit"
    );

    // Make a small request for the limited agent
    let small_cost = TokenCost::from_cents(5);
    let usage = TokenUsage::new(10, 5);
    tracker_with_agent
        .record_spending(
            "limited-agent".to_string(),
            "kimi".to_string(),
            "moonshot-v1-32k".to_string(),
            usage,
            small_cost,
        )
        .await?;

    // Verify agent spending
    let agent_status = tracker_with_agent.get_agent_status("limited-agent").await;
    assert!(agent_status.is_some());
    if let Some(status) = agent_status {
        assert_eq!(status.session_spent, small_cost);
    }

    println!("\nAll budget tracking tests passed!");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires MOONSHOT_API_KEY"]
async fn test_kimi_budget_model_selection() -> Result<(), Box<dyn std::error::Error>> {
    // Skip if API key not set
    if std::env::var("MOONSHOT_API_KEY").is_err() {
        eprintln!("MOONSHOT_API_KEY not set, skipping model selection test");
        return Ok(());
    }

    // Create budget manager
    let budget_config = BudgetConfig {
        limits: BudgetLimits {
            session_limit: Some(TokenCost::from_cents(50)), // 50 cents limit
            ..Default::default()
        },
        ..Default::default()
    };

    let budget_manager = Arc::new(BudgetManager::new(budget_config).await?);
    let policy = budget_manager.policy();

    // Test model selection based on budget constraints
    use arkavo_budget::policy::{SelectionCriteria, SelectionPriority};
    use arkavo_dataflow::nodes::model_registry::{ModelCapabilities, ModelInfo};
    use chrono::Utc;

    // Create test models including Kimi models
    let models = vec![
        ModelInfo {
            model_id: "moonshot-v1-32k".to_string(),
            provider_name: "kimi".to_string(),
            display_name: "Kimi 32K".to_string(),
            description: None,
            context_window: Some(32000),
            max_output_tokens: Some(8192),
            capabilities: ModelCapabilities {
                supports_streaming: true,
                supports_function_calling: false,
                supports_vision: false,
                supports_code_execution: false,
                input_modalities: vec!["text".to_string()],
                output_modalities: vec!["text".to_string()],
                languages: vec!["en".to_string(), "zh".to_string()],
                specializations: vec!["general".to_string()],
            },
            tags: vec!["general".to_string()],
            version: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: None,
            pricing: None, // Will use budget module's pricing
        },
        ModelInfo {
            model_id: "moonshot-v1-128k".to_string(),
            provider_name: "kimi".to_string(),
            display_name: "Kimi 128K".to_string(),
            description: None,
            context_window: Some(128000),
            max_output_tokens: Some(8192),
            capabilities: ModelCapabilities {
                supports_streaming: true,
                supports_function_calling: false,
                supports_vision: false,
                supports_code_execution: false,
                input_modalities: vec!["text".to_string()],
                output_modalities: vec!["text".to_string()],
                languages: vec!["en".to_string(), "zh".to_string()],
                specializations: vec!["general".to_string(), "long-context".to_string()],
            },
            tags: vec!["general".to_string(), "long-context".to_string()],
            version: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: None,
            pricing: None,
        },
    ];

    // Test 1: Select cheapest model
    let criteria = SelectionCriteria {
        required_capabilities: vec!["streaming".to_string()],
        preferred_providers: vec!["kimi".to_string()],
        max_acceptable_cost: Some(TokenCost::from_cents(20)),
        estimated_input_tokens: 1000,
        estimated_output_tokens: 500,
        priority: SelectionPriority::LowestCost,
    };

    let selected = policy.select_model(&criteria, TokenCost::from_cents(50), &models)?;

    assert!(selected.is_some(), "Should select a model");
    let candidate = selected.unwrap();
    println!(
        "Selected model for lowest cost: {} (${:.4})",
        candidate.model_id,
        candidate.estimated_cost.as_dollars()
    );
    assert_eq!(
        candidate.model_id, "moonshot-v1-32k",
        "Should select cheaper 32k model"
    );

    // Test 2: Select model with long context requirement
    let criteria = SelectionCriteria {
        required_capabilities: vec!["long-context".to_string()],
        preferred_providers: vec!["kimi".to_string()],
        max_acceptable_cost: None,
        estimated_input_tokens: 50000, // Requires large context
        estimated_output_tokens: 2000,
        priority: SelectionPriority::BestPerformance,
    };

    let selected = policy.select_model(&criteria, TokenCost::from_cents(100), &models)?;

    assert!(selected.is_some(), "Should select a model");
    let candidate = selected.unwrap();
    println!(
        "Selected model for long context: {} (${:.4})",
        candidate.model_id,
        candidate.estimated_cost.as_dollars()
    );
    assert_eq!(
        candidate.model_id, "moonshot-v1-128k",
        "Should select 128k model for long context"
    );

    // Test 3: Budget too small for any model
    let criteria = SelectionCriteria {
        required_capabilities: vec![],
        preferred_providers: vec!["kimi".to_string()],
        max_acceptable_cost: Some(TokenCost::from_cents(1)), // Very small budget
        estimated_input_tokens: 10000,
        estimated_output_tokens: 5000,
        priority: SelectionPriority::LowestCost,
    };

    let selected = policy.select_model(&criteria, TokenCost::from_cents(1), &models)?;

    assert!(
        selected.is_none(),
        "Should not select any model when budget is too small"
    );

    println!("\nAll model selection tests passed!");
    Ok(())
}
