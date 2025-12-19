//! Integration test for deliberation with Ministral 3B Reasoning model

use arkavo_llm::{LlamaCppProvider, Message, Provider, Role};
use arkavo_router::deliberation::{DeliberationConfig, Deliberator};
use std::sync::Arc;

const MINISTRAL_MODEL_PATH: &str = "/Users/paul/.cache/huggingface/hub/models--mistralai--Ministral-3-3B-Reasoning-2512-GGUF/snapshots/3b993f103009368df16c58fef4ad589cedf1bf24/Ministral-3-3B-Reasoning-2512-Q4_K_M.gguf";

#[tokio::test]
#[ignore] // Run with: cargo test -p arkavo-router --test deliberation_test -- --ignored --nocapture
async fn test_deliberation_with_ministral_3b() {
    // Check if model exists
    if !std::path::Path::new(MINISTRAL_MODEL_PATH).exists() {
        eprintln!("Ministral 3B model not found at: {}", MINISTRAL_MODEL_PATH);
        eprintln!("Download with: huggingface-cli download mistralai/Ministral-3-3B-Reasoning-2512-GGUF");
        return;
    }

    println!("Loading Ministral 3B Reasoning model...");
    let provider = LlamaCppProvider::new(
        "ministral-3b-reasoning".to_string(),
        MINISTRAL_MODEL_PATH.to_string(),
    )
    .expect("Failed to load Ministral 3B model");

    let provider: Arc<dyn Provider> = Arc::new(provider);

    // Create deliberator with critique enabled but no external judge
    let config = DeliberationConfig {
        enable_thinking: true,
        enable_critique: true,
        enable_judge: false,
        max_rounds: 2,
    };

    let deliberator = Deliberator::new(provider, config);

    // Test task: Simple reasoning question
    let task = "What is 15% of 80?";
    let messages = vec![Message {
        role: Role::User,
        content: task.to_string(),
        images: None,
    }];

    println!("\nTask: {}", task);
    println!("Running deliberation...\n");

    let result = deliberator
        .deliberate(task, messages, &[], None)
        .await
        .expect("Deliberation failed");

    println!("=== Deliberation Result ===");
    println!("Iterations: {}", result.iterations);
    println!("Confidence: {:.2}", result.confidence);
    if let Some(ref thinking) = result.thinking {
        println!("\nThinking: {}", thinking);
    }
    if let Some(ref critique) = result.critique {
        println!("\nCritique: {}", critique);
    }
    println!("\nFinal Response:\n{}", result.final_response);
    println!("===========================\n");

    // Basic assertions
    assert!(!result.final_response.is_empty(), "Response should not be empty");
    assert!(result.iterations >= 1, "Should have at least 1 iteration");
}

#[tokio::test]
#[ignore]
async fn test_deliberation_tool_error_scenario() {
    if !std::path::Path::new(MINISTRAL_MODEL_PATH).exists() {
        eprintln!("Ministral 3B model not found, skipping test");
        return;
    }

    println!("Loading Ministral 3B Reasoning model...");
    let provider = LlamaCppProvider::new(
        "ministral-3b-reasoning".to_string(),
        MINISTRAL_MODEL_PATH.to_string(),
    )
    .expect("Failed to load Ministral 3B model");

    let provider: Arc<dyn Provider> = Arc::new(provider);

    let config = DeliberationConfig {
        enable_thinking: true,
        enable_critique: true,
        enable_judge: false,
        max_rounds: 2,
    };

    let deliberator = Deliberator::new(provider, config);

    // Simulate a scenario where user asked to send a task and it failed
    let task = "I tried to send a task to agent 'test-agent' but it failed with 'Agent not found'. \
                What should I do?";

    let messages = vec![Message {
        role: Role::User,
        content: task.to_string(),
        images: None,
    }];

    println!("\nTask: {}", task);
    println!("Running deliberation...\n");

    let result = deliberator
        .deliberate(task, messages, &[], None)
        .await
        .expect("Deliberation failed");

    println!("=== Deliberation Result ===");
    println!("Iterations: {}", result.iterations);
    println!("Confidence: {:.2}", result.confidence);
    println!("\nFinal Response:\n{}", result.final_response);
    println!("===========================\n");

    // The response should acknowledge the error and suggest solutions
    let response_lower = result.final_response.to_lowercase();
    let mentions_error = response_lower.contains("error")
        || response_lower.contains("not found")
        || response_lower.contains("agent")
        || response_lower.contains("check");

    assert!(mentions_error, "Response should address the tool error");
}
