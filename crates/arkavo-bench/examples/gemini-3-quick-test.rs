//! Gemini 3 Pro Preview Quick Validation Test
//!
//! This example validates the new Gemini 3 Pro Preview model with:
//! 1. Lightweight SWE-bench test (1 instance)
//! 2. Coding exercise (algorithm challenge)
//! 3. Performance metrics (TTFT, latency, tokens, cost)
//!
//! This example uses #[tokio::main] which internally uses Runtime::block_on.
#![allow(clippy::disallowed_methods)]

use arkavo_bench::SweBenchTool;
use arkavo_gemini::RestClient;
use arkavo_mcp::Tool;
use serde_json::json;
use std::env;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY environment variable not set");
    let model_id = "models/gemini-3-pro-preview";

    println!("🎯 Gemini 3 Pro Preview - Quick Validation Test");
    println!("{}", "=".repeat(60));
    println!("Model: {}", model_id);
    println!("Tests: SWE-bench (1 instance) + Coding Exercise\n");

    let client = RestClient::new(&api_key, model_id);

    println!("📊 Test 1: SWE-bench Lite Instance");
    println!("{}", "-".repeat(60));

    run_swe_bench_test(&client, model_id).await?;

    println!("\n\n📊 Test 2: Coding Exercise - Two Sum Algorithm");
    println!("{}", "-".repeat(60));

    run_coding_exercise_two_sum(&client, model_id).await?;

    println!("\n\n📊 Test 3: Coding Exercise - Fibonacci Sequence");
    println!("{}", "-".repeat(60));

    run_coding_exercise_fibonacci(&client, model_id).await?;

    println!("\n\n🎉 Gemini 3 Pro Preview Validation Complete!");
    println!("\nResults saved to:");
    println!("  - /tmp/gemini-3-swe-bench.json");
    println!("  - /tmp/gemini-3-coding-exercises.json");

    Ok(())
}

async fn run_swe_bench_test(
    client: &RestClient,
    model_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let bench_tool = SweBenchTool::new();

    println!("🔄 Loading 1 SWE-bench Lite instance...");
    let load_params = json!({
        "action": "load",
        "subset": "lite",
        "limit": 1
    });

    let instances_result = bench_tool
        .execute(load_params)
        .await
        .map_err(|e| format!("Failed to load instances: {}", e))?;
    let instances = instances_result
        .get("instances")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "No instances loaded".to_string())?;

    if instances.is_empty() {
        return Err("No instances loaded".into());
    }

    let instance = &instances[0];
    let instance_id = instance
        .get("instance_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let problem = instance
        .get("problem_statement")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    println!("  Instance: {}", instance_id);
    println!(
        "  Problem: {}...\n",
        &problem.chars().take(80).collect::<String>()
    );

    let prompt = format!(
        "You are a software engineering expert solving GitHub issues.\n\n\
         Problem:\n{}\n\n\
         Provide a git diff patch that solves this issue. \
         Output ONLY the patch, no explanations.",
        problem
    );

    let start = Instant::now();
    let mut ttft_measured = false;
    let mut ttft = 0u64;

    let mut stream = client.stream_generate_content(&prompt, None).await?;
    let mut full_text = String::new();

    while let Some(chunk_result) = stream.next().await {
        match chunk_result {
            Ok(chunk) => {
                if !ttft_measured && chunk.text.is_some() {
                    ttft = start.elapsed().as_millis() as u64;
                    ttft_measured = true;
                    println!("  ⚡ Time to First Token (TTFT): {}ms", ttft);
                }
                if let Some(text) = chunk.text {
                    full_text.push_str(&text);
                }
            }
            Err(e) => {
                return Err(format!("Stream error: {}", e).into());
            }
        }
    }

    let elapsed = start.elapsed();
    let tokens_estimate = full_text.len() / 4;
    let cost_estimate = estimate_cost(model_id, tokens_estimate);

    println!("\n  ✅ Results:");
    println!("     Total time: {:?}", elapsed);
    println!("     TTFT: {}ms", ttft);
    println!("     Solution length: {} chars", full_text.len());
    println!("     Est. tokens: {}", tokens_estimate);
    println!("     Est. cost: ${:.4}", cost_estimate);

    if full_text.len() < 300 {
        println!("\n  📄 Full solution:\n{}", full_text);
    } else {
        println!(
            "\n  📄 Preview: {}...",
            &full_text.chars().take(200).collect::<String>()
        );
    }

    let result = json!({
        "instance_id": instance_id,
        "elapsed_ms": elapsed.as_millis(),
        "ttft_ms": ttft,
        "solution_length": full_text.len(),
        "tokens_estimate": tokens_estimate,
        "cost_estimate": cost_estimate,
        "solution": full_text
    });

    tokio::fs::write(
        "/tmp/gemini-3-swe-bench.json",
        serde_json::to_string_pretty(&result)?,
    )
    .await?;
    println!("\n  📁 Saved to /tmp/gemini-3-swe-bench.json");

    Ok(())
}

async fn run_coding_exercise_two_sum(
    client: &RestClient,
    model_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let prompt = r#"Write a Rust function that solves the Two Sum problem:

Given an array of integers nums and an integer target, return indices of the two numbers
such that they add up to target. You may assume that each input would have exactly one solution.

Example:
Input: nums = [2,7,11,15], target = 9
Output: [0,1] (because nums[0] + nums[1] = 9)

Provide ONLY the complete Rust function with proper error handling. Include the function signature:
pub fn two_sum(nums: Vec<i32>, target: i32) -> Option<(usize, usize)>

No explanations, just the code."#;

    println!("  Prompt: Two Sum algorithm implementation\n");

    let start = Instant::now();
    let mut ttft = 0u64;
    let mut ttft_measured = false;

    let mut stream = client.stream_generate_content(prompt, None).await?;
    let mut full_text = String::new();

    while let Some(chunk_result) = stream.next().await {
        match chunk_result {
            Ok(chunk) => {
                if !ttft_measured && chunk.text.is_some() {
                    ttft = start.elapsed().as_millis() as u64;
                    ttft_measured = true;
                    println!("  ⚡ TTFT: {}ms", ttft);
                }
                if let Some(text) = chunk.text {
                    full_text.push_str(&text);
                }
            }
            Err(e) => {
                return Err(format!("Stream error: {}", e).into());
            }
        }
    }

    let elapsed = start.elapsed();
    let tokens_estimate = full_text.len() / 4;
    let cost_estimate = estimate_cost(model_id, tokens_estimate);

    println!("\n  ✅ Results:");
    println!("     Total time: {:?}", elapsed);
    println!("     TTFT: {}ms", ttft);
    println!("     Code length: {} chars", full_text.len());
    println!("     Est. tokens: {}", tokens_estimate);
    println!("     Est. cost: ${:.4}", cost_estimate);

    println!("\n  📄 Generated Code:\n{}", full_text);

    let result = json!({
        "exercise": "two_sum",
        "elapsed_ms": elapsed.as_millis(),
        "ttft_ms": ttft,
        "code_length": full_text.len(),
        "tokens_estimate": tokens_estimate,
        "cost_estimate": cost_estimate,
        "code": full_text
    });

    save_coding_exercise_result(result).await?;

    Ok(())
}

async fn run_coding_exercise_fibonacci(
    client: &RestClient,
    model_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let prompt = r#"Write an efficient Rust function that generates the first n Fibonacci numbers:

The Fibonacci sequence starts with 0, 1, and each subsequent number is the sum of the previous two.

Example:
Input: n = 7
Output: [0, 1, 1, 2, 3, 5, 8]

Provide ONLY the complete Rust function. Include the function signature:
pub fn fibonacci(n: usize) -> Vec<u64>

Use an efficient iterative approach. No explanations, just the code."#;

    println!("  Prompt: Fibonacci sequence generation\n");

    let start = Instant::now();
    let mut ttft = 0u64;
    let mut ttft_measured = false;

    let mut stream = client.stream_generate_content(prompt, None).await?;
    let mut full_text = String::new();

    while let Some(chunk_result) = stream.next().await {
        match chunk_result {
            Ok(chunk) => {
                if !ttft_measured && chunk.text.is_some() {
                    ttft = start.elapsed().as_millis() as u64;
                    ttft_measured = true;
                    println!("  ⚡ TTFT: {}ms", ttft);
                }
                if let Some(text) = chunk.text {
                    full_text.push_str(&text);
                }
            }
            Err(e) => {
                return Err(format!("Stream error: {}", e).into());
            }
        }
    }

    let elapsed = start.elapsed();
    let tokens_estimate = full_text.len() / 4;
    let cost_estimate = estimate_cost(model_id, tokens_estimate);

    println!("\n  ✅ Results:");
    println!("     Total time: {:?}", elapsed);
    println!("     TTFT: {}ms", ttft);
    println!("     Code length: {} chars", full_text.len());
    println!("     Est. tokens: {}", tokens_estimate);
    println!("     Est. cost: ${:.4}", cost_estimate);

    println!("\n  📄 Generated Code:\n{}", full_text);

    let result = json!({
        "exercise": "fibonacci",
        "elapsed_ms": elapsed.as_millis(),
        "ttft_ms": ttft,
        "code_length": full_text.len(),
        "tokens_estimate": tokens_estimate,
        "cost_estimate": cost_estimate,
        "code": full_text
    });

    save_coding_exercise_result(result).await?;

    Ok(())
}

async fn save_coding_exercise_result(
    result: serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let file_path = "/tmp/gemini-3-coding-exercises.json";

    let mut results = if let Ok(existing) = tokio::fs::read_to_string(file_path).await {
        serde_json::from_str::<Vec<serde_json::Value>>(&existing).unwrap_or_else(|_| vec![])
    } else {
        vec![]
    };

    results.push(result);

    tokio::fs::write(file_path, serde_json::to_string_pretty(&results)?).await?;
    println!("  📁 Saved to {}", file_path);

    Ok(())
}

fn estimate_cost(model: &str, tokens: usize) -> f64 {
    let (input_cost, output_cost) = match model {
        "models/gemini-flash-latest" => (0.000075, 0.0003),
        "models/gemini-3-pro-preview" => (0.00125, 0.005),
        _ => (0.0001, 0.0004),
    };

    let avg_cost_per_1k = (input_cost + output_cost) / 2.0;
    (tokens as f64 / 1000.0) * avg_cost_per_1k
}
