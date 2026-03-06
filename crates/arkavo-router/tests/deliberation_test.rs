//! Integration tests for deliberation with local GGUF models
//!
//! These tests require the `llama-cpp` feature and local GGUF models.
//! Run with: cargo test -p arkavo-router --features llama-cpp --test deliberation_test -- --ignored --nocapture

#![cfg(feature = "llama-cpp")]

use arkavo_llm::{LlamaCppProvider, Message, Provider, Role};
use arkavo_router::deliberation::{DeliberationConfig, Deliberator};
use arkavo_router::model_discovery;
use std::sync::Arc;

/// Find Ministral 3B model dynamically from HuggingFace cache
async fn find_ministral_model() -> Option<String> {
    // Try Instruct variant first, then Reasoning variant
    if let Ok(path) = model_discovery::find_gguf_model(
        "mistralai/Ministral-3-3B-Instruct-2512-GGUF",
        "Ministral-3-3B-Instruct-2512-Q4_K_M.gguf",
    )
    .await
    {
        return Some(path.to_string_lossy().to_string());
    }
    if let Ok(path) = model_discovery::find_gguf_model(
        "mistralai/Ministral-3-3B-Reasoning-2512-GGUF",
        "Ministral-3-3B-Reasoning-2512-Q4_K_M.gguf",
    )
    .await
    {
        return Some(path.to_string_lossy().to_string());
    }
    None
}

/// Find Qwen3 model dynamically from HuggingFace cache
async fn find_qwen3_model() -> Option<String> {
    use arkavo_router::decision::ModelChoice;
    let repo = ModelChoice::LocalQwen3.repo_id().unwrap();
    let file = ModelChoice::LocalQwen3.gguf_filename().unwrap();
    if let Ok(path) = model_discovery::find_gguf_model(repo, file).await {
        return Some(path.to_string_lossy().to_string());
    }
    None
}

#[tokio::test]
#[ignore = "requires local model file"]
async fn test_deliberation_with_ministral_3b() {
    // Find model dynamically from HuggingFace cache
    let model_path = match find_ministral_model().await {
        Some(path) => path,
        None => {
            eprintln!("Ministral 3B model not found in HuggingFace cache");
            eprintln!(
                "Download with: huggingface-cli download mistralai/Ministral-3-3B-Instruct-2512-GGUF"
            );
            return;
        }
    };

    println!("Loading Ministral 3B model from: {}", model_path);
    let provider = LlamaCppProvider::new("ministral-3b".to_string(), model_path)
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
    assert!(
        !result.final_response.is_empty(),
        "Response should not be empty"
    );
    assert!(result.iterations >= 1, "Should have at least 1 iteration");
}

#[tokio::test]
#[ignore = "requires local model file"]
async fn test_deliberation_tool_error_scenario() {
    let model_path = match find_ministral_model().await {
        Some(path) => path,
        None => {
            eprintln!("Ministral 3B model not found, skipping test");
            return;
        }
    };

    println!("Loading Ministral 3B model from: {}", model_path);
    let provider = LlamaCppProvider::new("ministral-3b".to_string(), model_path)
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

#[tokio::test]
#[ignore = "requires local model file"]
async fn test_qwen3_math() {
    let model_path = match find_qwen3_model().await {
        Some(path) => path,
        None => {
            eprintln!("Qwen3-0.6B model not found in HuggingFace cache");
            eprintln!("Download with: huggingface-cli download unsloth/Qwen3.5-0.8B-GGUF");
            return;
        }
    };

    println!("Loading Qwen3-0.6B model from: {}", model_path);
    let provider = LlamaCppProvider::new("qwen3.5-0.8b".to_string(), model_path)
        .expect("Failed to load Qwen3 model");

    let provider: Arc<dyn Provider> = Arc::new(provider);

    let config = DeliberationConfig {
        enable_thinking: true,
        enable_critique: false, // Skip critique for speed
        enable_judge: false,
        max_rounds: 1,
    };

    let deliberator = Deliberator::new(provider, config);

    let task = "What is 7 * 8?";
    let messages = vec![Message {
        role: Role::User,
        content: task.to_string(),
        images: None,
    }];

    println!("\nTask: {}", task);
    println!("Running with Qwen3-0.6B...\n");

    let result = deliberator
        .deliberate(task, messages, &[], None)
        .await
        .expect("Deliberation failed");

    println!("=== Qwen3-0.6B Result ===");
    println!("Iterations: {}", result.iterations);
    println!("\nResponse:\n{}", result.final_response);
    println!("=========================\n");

    assert!(
        !result.final_response.is_empty(),
        "Response should not be empty"
    );
}

#[tokio::test]
#[ignore = "requires local model file"]
async fn test_qwen3_coding() {
    let model_path = match find_qwen3_model().await {
        Some(path) => path,
        None => {
            eprintln!("Qwen3-0.6B model not found, skipping test");
            return;
        }
    };

    println!("Loading Qwen3-0.6B model from: {}", model_path);
    let provider = LlamaCppProvider::new("qwen3.5-0.8b".to_string(), model_path)
        .expect("Failed to load Qwen3 model");

    let provider: Arc<dyn Provider> = Arc::new(provider);

    let config = DeliberationConfig {
        enable_thinking: true,
        enable_critique: false,
        enable_judge: false,
        max_rounds: 1,
    };

    let deliberator = Deliberator::new(provider, config);

    let task = "Write a Python function to check if a number is prime.";
    let messages = vec![Message {
        role: Role::User,
        content: task.to_string(),
        images: None,
    }];

    println!("\nTask: {}", task);
    println!("Running with Qwen3-0.6B...\n");

    let result = deliberator
        .deliberate(task, messages, &[], None)
        .await
        .expect("Deliberation failed");

    println!("=== Qwen3-0.6B Coding Result ===");
    println!("\nResponse:\n{}", result.final_response);
    println!("================================\n");

    let response_lower = result.final_response.to_lowercase();
    let has_code = response_lower.contains("def ")
        || response_lower.contains("function")
        || response_lower.contains("prime");

    assert!(has_code, "Response should contain code or mention prime");
}
