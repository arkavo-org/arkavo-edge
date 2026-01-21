//! Integration tests for tool format learning with local GGUF models
//!
//! These tests validate that the HRM learning system correctly tracks which
//! tool call formats work best for different models.
//!
//! Run with: cargo test -p arkavo-router --features llama-cpp --test format_learning_test -- --ignored --nocapture

#![cfg(feature = "llama-cpp")]

use arkavo_llm::{LlamaCppProvider, LocalToolFormat, Message, Provider, Role, SamplingConfig};
use arkavo_router::learning::{BurstFeedback, LearningModule, ToolCallFormat};
use arkavo_router::model_discovery;
use std::time::Instant;
use uuid::Uuid;

/// Tool schema for testing
const TEST_TOOL_SCHEMA: &str = r#"
You have access to the following tool:

get_weather: Get the current weather for a location
  Parameters:
    - location (string, required): The city name
    - unit (string, optional): "celsius" or "fahrenheit"

To use a tool, output a code block with the tool name:
```get_weather
location: San Francisco
unit: celsius
```
"#;

/// Find Ministral 3B model dynamically from HuggingFace cache
async fn find_ministral_model() -> Option<String> {
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
    if let Ok(path) =
        model_discovery::find_gguf_model("Qwen/Qwen3-0.6B-GGUF", "Qwen3-0.6B-Q8_0.gguf").await
    {
        return Some(path.to_string_lossy().to_string());
    }
    None
}

/// Check if response contains a valid fence-format tool call
fn has_fence_tool_call(response: &str) -> bool {
    // Look for ```tool_name pattern
    let fence_pattern = regex::Regex::new(r"```([a-z_]+)\n").unwrap();
    fence_pattern.is_match(response)
}

/// Check if response contains a valid XML-format tool call
fn has_xml_tool_call(response: &str) -> bool {
    response.contains("<tool_call>") || response.contains("<get_weather>")
}

/// Check if response contains a valid JSON-format tool call
fn has_json_tool_call(response: &str) -> bool {
    response.contains(r#""name":"#) || response.contains(r#""tool_call":"#)
}

/// Detect which format was used in the response
fn detect_format(response: &str) -> Option<ToolCallFormat> {
    if has_fence_tool_call(response) {
        Some(ToolCallFormat::Fence)
    } else if has_xml_tool_call(response) {
        Some(ToolCallFormat::Xml)
    } else if has_json_tool_call(response) {
        Some(ToolCallFormat::Json)
    } else {
        None
    }
}

/// Test format learning with a specific model and format
async fn test_format_with_model(
    provider: &dyn Provider,
    model_id: &str,
    format: ToolCallFormat,
    learning: &LearningModule,
) -> (bool, u64) {
    let format_instruction = match format {
        ToolCallFormat::Fence => "Use fence format: ```tool_name\\nkey: value\\n```",
        ToolCallFormat::Xml => {
            "Use XML format: <tool_call><name>tool</name><arguments>{}</arguments></tool_call>"
        }
        ToolCallFormat::Json => r#"Use JSON format: {"name": "tool", "arguments": {...}}"#,
        ToolCallFormat::PythonCall => "Use Python format: tool_name(key=value)",
    };

    let prompt = format!(
        "{}\n\n{}\n\nUser: What's the weather in Tokyo?\n\nAssistant:",
        TEST_TOOL_SCHEMA, format_instruction
    );

    let messages = vec![Message {
        role: Role::User,
        content: prompt,
        images: None,
    }];

    let start = Instant::now();
    let result = provider.complete_with_options(messages, Some(256)).await;
    let latency_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(response) => {
            let detected = detect_format(&response);
            let success = detected == Some(format);

            println!(
                "  {} format test: {} (detected: {:?}, latency: {}ms)",
                format,
                if success { "PASS" } else { "FAIL" },
                detected,
                latency_ms
            );

            if !success {
                let snippet_len = response.len().min(200);
                println!("    Response snippet: {}...", &response[..snippet_len]);
            }

            // Record feedback using category key convention
            let feedback = if success {
                BurstFeedback::success(Uuid::new_v4(), format.to_category_key(), latency_ms)
            } else {
                BurstFeedback::failure(Uuid::new_v4(), format.to_category_key(), latency_ms)
            };

            learning.immediate_update(model_id, &feedback).await;

            (success, latency_ms)
        }
        Err(e) => {
            println!("  {} format test: ERROR - {}", format, e);

            // Record failure
            let feedback =
                BurstFeedback::failure(Uuid::new_v4(), format.to_category_key(), latency_ms);
            learning.immediate_update(model_id, &feedback).await;

            (false, latency_ms)
        }
    }
}

#[tokio::test]
#[ignore = "requires llama-cpp feature and local model"]
async fn test_format_learning_ministral() {
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

    let config = SamplingConfig {
        temperature: 0.1, // Low temp for deterministic tool calls
        top_p: 0.9,
        top_k: 40,
        max_tokens: 256,
        seed: 42,
        debug: false,
        tool_format: LocalToolFormat::Fence,
    };

    let provider =
        LlamaCppProvider::new_with_config("ministral-3b".to_string(), model_path, None, config)
            .expect("Failed to load Ministral 3B model");

    let learning = LearningModule::new();
    let model_id = "ministral-3b";

    println!("\n=== Testing Ministral 3B Format Learning ===\n");

    // Test each format multiple times
    let formats = [
        ToolCallFormat::Fence,
        ToolCallFormat::Xml,
        ToolCallFormat::Json,
    ];

    let mut results: Vec<(ToolCallFormat, u32, u32)> = Vec::new();

    for format in &formats {
        let mut successes = 0u32;
        let mut total = 0u32;

        println!("Testing {} format (3 iterations):", format);

        for i in 0..3 {
            let (success, _latency) =
                test_format_with_model(&provider, model_id, *format, &learning).await;
            if success {
                successes += 1;
            }
            total += 1;
            println!(
                "  Iteration {}: {}",
                i + 1,
                if success { "success" } else { "failure" }
            );
        }

        results.push((*format, successes, total));
        println!();
    }

    // Check learning results
    println!("=== Format Learning Results ===\n");

    let stats = learning.get_format_stats(model_id).await;
    for (format, successes, total) in &results {
        let success_rate = *successes as f64 / *total as f64;
        let learned = stats.get(format).map(|(ev, obs)| (*ev, *obs));
        println!(
            "{}: {}/{} success rate={:.0}%, learned={:?}",
            format,
            successes,
            total,
            success_rate * 100.0,
            learned
        );
    }

    // Sample format to see what learning recommends
    println!("\nThompson Sampling format selection (5 samples):");
    for i in 0..5 {
        let (format, score) = learning.sample_format(model_id).await;
        println!("  Sample {}: {} (score: {:.3})", i + 1, format, score);
    }
}

#[tokio::test]
#[ignore = "requires llama-cpp feature and local model"]
async fn test_format_learning_qwen() {
    let model_path = match find_qwen3_model().await {
        Some(path) => path,
        None => {
            eprintln!("Qwen3 0.6B model not found in HuggingFace cache");
            eprintln!("Download with: huggingface-cli download Qwen/Qwen3-0.6B-GGUF");
            return;
        }
    };

    println!("Loading Qwen3 0.6B model from: {}", model_path);

    let config = SamplingConfig {
        temperature: 0.1,
        top_p: 0.9,
        top_k: 40,
        max_tokens: 256,
        seed: 42,
        debug: false,
        tool_format: LocalToolFormat::Fence,
    };

    let provider =
        LlamaCppProvider::new_with_config("qwen3-0.6b".to_string(), model_path, None, config)
            .expect("Failed to load Qwen3 0.6B model");

    let learning = LearningModule::new();
    let model_id = "qwen3-0.6b";

    println!("\n=== Testing Qwen3 0.6B Format Learning ===\n");

    // Test each format multiple times
    let formats = [
        ToolCallFormat::Fence,
        ToolCallFormat::Xml,
        ToolCallFormat::Json,
    ];

    let mut results: Vec<(ToolCallFormat, u32, u32)> = Vec::new();

    for format in &formats {
        let mut successes = 0u32;
        let mut total = 0u32;

        println!("Testing {} format (3 iterations):", format);

        for i in 0..3 {
            let (success, _latency) =
                test_format_with_model(&provider, model_id, *format, &learning).await;
            if success {
                successes += 1;
            }
            total += 1;
            println!(
                "  Iteration {}: {}",
                i + 1,
                if success { "success" } else { "failure" }
            );
        }

        results.push((*format, successes, total));
        println!();
    }

    // Check learning results
    println!("=== Format Learning Results ===\n");

    let stats = learning.get_format_stats(model_id).await;
    for (format, successes, total) in &results {
        let success_rate = *successes as f64 / *total as f64;
        let learned = stats.get(format).map(|(ev, obs)| (*ev, *obs));
        println!(
            "{}: {}/{} success rate={:.0}%, learned={:?}",
            format,
            successes,
            total,
            success_rate * 100.0,
            learned
        );
    }

    // Sample format to see what learning recommends
    println!("\nThompson Sampling format selection (5 samples):");
    for i in 0..5 {
        let (format, score) = learning.sample_format(model_id).await;
        println!("  Sample {}: {} (score: {:.3})", i + 1, format, score);
    }
}

#[tokio::test]
#[ignore = "requires llama-cpp feature and local model"]
async fn test_format_learning_comparison() {
    println!("\n=== Cross-Model Format Learning Comparison ===\n");

    let learning = LearningModule::new();

    // Test with Ministral if available
    if let Some(model_path) = find_ministral_model().await {
        println!("Testing Ministral 3B...");
        let config = SamplingConfig {
            temperature: 0.1,
            top_p: 0.9,
            top_k: 40,
            max_tokens: 256,
            seed: 42,
            debug: false,
            tool_format: LocalToolFormat::Fence,
        };

        if let Ok(provider) =
            LlamaCppProvider::new_with_config("ministral-3b".to_string(), model_path, None, config)
        {
            for format in ToolCallFormat::all() {
                let _ = test_format_with_model(&provider, "ministral-3b", *format, &learning).await;
            }
        }
    }

    // Test with Qwen if available
    if let Some(model_path) = find_qwen3_model().await {
        println!("\nTesting Qwen3 0.6B...");
        let config = SamplingConfig {
            temperature: 0.1,
            top_p: 0.9,
            top_k: 40,
            max_tokens: 256,
            seed: 42,
            debug: false,
            tool_format: LocalToolFormat::Fence,
        };

        if let Ok(provider) =
            LlamaCppProvider::new_with_config("qwen3-0.6b".to_string(), model_path, None, config)
        {
            for format in ToolCallFormat::all() {
                let _ = test_format_with_model(&provider, "qwen3-0.6b", *format, &learning).await;
            }
        }
    }

    // Print comparison
    println!("\n=== Final Format Statistics ===\n");

    for model_id in &["ministral-3b", "qwen3-0.6b"] {
        let stats = learning.get_format_stats(model_id).await;
        if !stats.is_empty() {
            println!("{}:", model_id);
            for format in ToolCallFormat::all() {
                if let Some((ev, obs)) = stats.get(format) {
                    println!(
                        "  {}: expected_value={:.3}, observations={}",
                        format, ev, obs
                    );
                }
            }

            let (best_format, score) = learning.sample_format(model_id).await;
            println!("  Recommended: {} (score: {:.3})\n", best_format, score);
        }
    }
}
