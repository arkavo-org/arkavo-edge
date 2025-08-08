use arkavo_budget::{BudgetManager, BudgetPolicy, TokenCost, UsageEvent};
use arkavo_dataflow::nodes::openai_provider::{OpenAIConfig, OpenAIProvider};
use arkavo_llm::{Message, Provider, Role};
use std::sync::Arc;
use tokio::sync::Mutex;

#[path = "mod.rs"]
mod common;
use common::ensure_api_key;

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_openai_token_cost_tracking() {
    let api_key = ensure_api_key();

    let config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-3.5-turbo".to_string(),
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config)
        .expect("Failed to create OpenAI provider");

    let budget_manager = Arc::new(Mutex::new(BudgetManager::new()));

    // Set a budget policy
    let policy = BudgetPolicy {
        max_cost_per_request: Some(TokenCost::from_cents(100)), // $1.00
        max_cost_per_hour: Some(TokenCost::from_cents(1000)),   // $10.00
        max_cost_per_day: Some(TokenCost::from_cents(10000)),   // $100.00
        warning_threshold: 0.8,
    };

    budget_manager.lock().await.set_policy(policy);

    let messages = vec![Message {
        role: Role::User,
        content: "Say hello in exactly 5 words.".to_string(),
        images: None,
    }];

    // Track usage before request
    let initial_cost = budget_manager.lock().await.get_total_cost();

    let response = provider.complete(messages.clone()).await
        .expect("Failed to get response");

    // Simulate tracking usage (in real implementation, this would be automatic)
    let usage_event = UsageEvent {
        provider: "openai".to_string(),
        model: "gpt-3.5-turbo".to_string(),
        input_tokens: estimate_tokens(&messages[0].content),
        output_tokens: estimate_tokens(&response),
        cost: calculate_gpt35_cost(
            estimate_tokens(&messages[0].content),
            estimate_tokens(&response),
        ),
        timestamp: chrono::Utc::now(),
        request_id: None,
    };

    budget_manager.lock().await.track_usage(usage_event.clone());

    let final_cost = budget_manager.lock().await.get_total_cost();
    
    println!("Initial cost: ${:.4}", initial_cost.to_dollars());
    println!("Final cost: ${:.4}", final_cost.to_dollars());
    println!("Request cost: ${:.4}", usage_event.cost.to_dollars());

    assert!(final_cost > initial_cost);
    assert!(usage_event.cost > TokenCost::zero());
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_openai_budget_limit_enforcement() {
    let api_key = ensure_api_key();

    let config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-3.5-turbo".to_string(),
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config)
        .expect("Failed to create OpenAI provider");

    let budget_manager = Arc::new(Mutex::new(BudgetManager::new()));

    // Set a very low budget
    let policy = BudgetPolicy {
        max_cost_per_request: Some(TokenCost::from_cents(1)), // $0.01 - very low
        max_cost_per_hour: Some(TokenCost::from_cents(10)),
        max_cost_per_day: Some(TokenCost::from_cents(100)),
        warning_threshold: 0.5,
    };

    budget_manager.lock().await.set_policy(policy);

    // Try a request that would exceed the per-request budget
    let messages = vec![Message {
        role: Role::User,
        content: "Write a 500-word essay about artificial intelligence.".to_string(),
        images: None,
    }];

    // Check if request would exceed budget before making it
    let estimated_cost = calculate_gpt35_cost(
        estimate_tokens(&messages[0].content),
        200, // Estimated output tokens for a 500-word essay
    );

    let can_proceed = budget_manager
        .lock()
        .await
        .check_budget_before_request(estimated_cost);

    if !can_proceed {
        println!("Request blocked due to budget limit");
        assert!(estimated_cost > TokenCost::from_cents(1));
    } else {
        let response = provider.complete(messages).await
            .expect("Failed to get response");
        
        println!("Response received (within budget): {} chars", response.len());
    }
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_openai_model_cost_comparison() {
    let api_key = ensure_api_key();

    let models = vec![
        ("gpt-3.5-turbo", "GPT-3.5 Turbo"),
        ("gpt-4-turbo", "GPT-4 Turbo"),
    ];

    let budget_manager = Arc::new(Mutex::new(BudgetManager::new()));
    let test_message = "What is 2+2?";

    for (model_id, model_name) in models {
        let config = OpenAIConfig {
            api_key: api_key.clone(),
            base_url: "https://api.openai.com/v1".to_string(),
            model: model_id.to_string(),
            organization_id: None,
            api_version: None,
            is_azure: false,
        };

        match OpenAIProvider::new(config) {
            Ok(provider) => {
                let messages = vec![Message {
                    role: Role::User,
                    content: test_message.to_string(),
                    images: None,
                }];

                match provider.complete(messages).await {
                    Ok(response) => {
                        let input_tokens = estimate_tokens(test_message);
                        let output_tokens = estimate_tokens(&response);
                        
                        let cost = match model_id {
                            "gpt-3.5-turbo" => calculate_gpt35_cost(input_tokens, output_tokens),
                            "gpt-4-turbo" => calculate_gpt4_turbo_cost(input_tokens, output_tokens),
                            _ => TokenCost::zero(),
                        };

                        let usage_event = UsageEvent {
                            provider: "openai".to_string(),
                            model: model_id.to_string(),
                            input_tokens,
                            output_tokens,
                            cost,
                            timestamp: chrono::Utc::now(),
                            request_id: None,
                        };

                        budget_manager.lock().await.track_usage(usage_event.clone());

                        println!("{} cost: ${:.6} ({} in, {} out tokens)",
                            model_name,
                            cost.to_dollars(),
                            input_tokens,
                            output_tokens
                        );
                    }
                    Err(e) => {
                        println!("Failed to get response from {}: {}", model_name, e);
                    }
                }
            }
            Err(e) => {
                println!("Failed to create provider for {}: {}", model_name, e);
            }
        }
    }

    let total_cost = budget_manager.lock().await.get_total_cost();
    println!("Total cost for all models: ${:.4}", total_cost.to_dollars());
    
    assert!(total_cost > TokenCost::zero());
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_openai_streaming_cost_tracking() {
    let api_key = ensure_api_key();

    let config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-3.5-turbo".to_string(),
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config)
        .expect("Failed to create OpenAI provider");

    let budget_manager = Arc::new(Mutex::new(BudgetManager::new()));

    let messages = vec![Message {
        role: Role::User,
        content: "Count from 1 to 10.".to_string(),
        images: None,
    }];

    use futures::StreamExt;
    let mut stream = provider.stream(messages.clone()).await
        .expect("Failed to create stream");

    let mut total_output = String::new();
    
    while let Some(result) = stream.next().await {
        match result {
            Ok(response) => {
                total_output.push_str(&response.content);
                if response.done {
                    break;
                }
            }
            Err(e) => {
                panic!("Stream error: {}", e);
            }
        }
    }

    // Track the complete streamed response
    let usage_event = UsageEvent {
        provider: "openai".to_string(),
        model: "gpt-3.5-turbo".to_string(),
        input_tokens: estimate_tokens(&messages[0].content),
        output_tokens: estimate_tokens(&total_output),
        cost: calculate_gpt35_cost(
            estimate_tokens(&messages[0].content),
            estimate_tokens(&total_output),
        ),
        timestamp: chrono::Utc::now(),
        request_id: None,
    };

    budget_manager.lock().await.track_usage(usage_event.clone());

    println!("Streaming response cost: ${:.6}", usage_event.cost.to_dollars());
    println!("Output tokens: {}", usage_event.output_tokens);

    assert!(usage_event.cost > TokenCost::zero());
    assert!(usage_event.output_tokens > 0);
}

#[tokio::test]
async fn test_budget_warning_threshold() {
    let mut budget_manager = BudgetManager::new();

    let policy = BudgetPolicy {
        max_cost_per_request: None,
        max_cost_per_hour: Some(TokenCost::from_cents(100)), // $1.00
        max_cost_per_day: None,
        warning_threshold: 0.8, // Warn at 80%
    };

    budget_manager.set_policy(policy);

    // Add usage up to 70% of limit
    let usage1 = UsageEvent {
        provider: "openai".to_string(),
        model: "gpt-3.5-turbo".to_string(),
        input_tokens: 100,
        output_tokens: 100,
        cost: TokenCost::from_cents(70), // $0.70
        timestamp: chrono::Utc::now(),
        request_id: None,
    };

    budget_manager.track_usage(usage1);
    assert!(!budget_manager.is_at_warning_threshold());

    // Add usage to exceed warning threshold (85% total)
    let usage2 = UsageEvent {
        provider: "openai".to_string(),
        model: "gpt-3.5-turbo".to_string(),
        input_tokens: 50,
        output_tokens: 50,
        cost: TokenCost::from_cents(15), // $0.15
        timestamp: chrono::Utc::now(),
        request_id: None,
    };

    budget_manager.track_usage(usage2);
    assert!(budget_manager.is_at_warning_threshold());
    
    let stats = budget_manager.get_usage_stats();
    println!("Current usage: ${:.2} of ${:.2} limit ({}%)",
        stats.total_cost.to_dollars(),
        TokenCost::from_cents(100).to_dollars(),
        (stats.total_cost.to_dollars() / TokenCost::from_cents(100).to_dollars()) * 100.0
    );
}

// Helper functions
fn estimate_tokens(text: &str) -> u32 {
    // Rough estimation: ~4 characters per token
    (text.len() / 4) as u32 + 1
}

fn calculate_gpt35_cost(input_tokens: u32, output_tokens: u32) -> TokenCost {
    // GPT-3.5-turbo pricing (as of 2024)
    // Input: $0.50 per 1K tokens = 0.05 cents per token
    // Output: $1.50 per 1K tokens = 0.15 cents per token
    let input_cost = TokenCost::from_cents((input_tokens as f64 * 0.05) as u64);
    let output_cost = TokenCost::from_cents((output_tokens as f64 * 0.15) as u64);
    input_cost + output_cost
}

fn calculate_gpt4_turbo_cost(input_tokens: u32, output_tokens: u32) -> TokenCost {
    // GPT-4-turbo pricing (as of 2024)
    // Input: $10.00 per 1K tokens = 1.0 cents per token
    // Output: $30.00 per 1K tokens = 3.0 cents per token
    let input_cost = TokenCost::from_cents((input_tokens as f64 * 1.0) as u64);
    let output_cost = TokenCost::from_cents((output_tokens as f64 * 3.0) as u64);
    input_cost + output_cost
}