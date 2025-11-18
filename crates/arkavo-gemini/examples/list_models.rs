//! List models example
//!
//! This example uses #[tokio::main] which internally uses Runtime::block_on.
#![allow(clippy::disallowed_methods)]

use arkavo_gemini::RestClient;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let api_key = env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY required");

    println!("Creating REST client...");
    let client = RestClient::new(api_key, "unused");

    println!("Listing available Google AI models...\n");
    let response = client.list_models(None).await?;

    println!("Found {} models:\n", response.models.len());
    println!(
        "{:<40} {:<15} {:<10} {:<10}",
        "Model Name", "Base Model", "Input", "Output"
    );
    println!("{}", "=".repeat(80));

    for model in &response.models {
        let base_model = model.base_model_id.as_deref().unwrap_or("N/A");
        let input_limit = model
            .input_token_limit
            .map(|n| n.to_string())
            .unwrap_or_else(|| "N/A".to_string());
        let output_limit = model
            .output_token_limit
            .map(|n| n.to_string())
            .unwrap_or_else(|| "N/A".to_string());

        println!(
            "{:<40} {:<15} {:<10} {:<10}",
            model.name, base_model, input_limit, output_limit
        );

        if let Some(methods) = &model.supported_generation_methods {
            println!("  Supported methods: {}", methods.join(", "));
        }
    }

    println!("\n{}", "=".repeat(80));

    let streaming_models: Vec<_> = response
        .models
        .iter()
        .filter(|m| {
            m.supported_generation_methods
                .as_ref()
                .map(|methods| methods.iter().any(|m| m == "streamGenerateContent"))
                .unwrap_or(false)
        })
        .collect();

    println!(
        "\nModels supporting streaming: {} of {}",
        streaming_models.len(),
        response.models.len()
    );

    for model in streaming_models.iter().take(5) {
        println!("  - {}", model.name);
    }

    // Check for Gemini 3 Pro Preview
    let gemini_3_preview = response.models.iter().find(|m| {
        m.name.contains("gemini-3-pro-preview") || m.name.contains("models/gemini-3-pro-preview")
    });

    if let Some(model) = gemini_3_preview {
        println!("\n{}", "=".repeat(80));
        println!("🎯 Gemini 3 Pro Preview detected!");
        println!("  Model: {}", model.name);
        if let Some(methods) = &model.supported_generation_methods {
            let has_streaming = methods.iter().any(|m| m == "streamGenerateContent");
            println!(
                "  Streaming: {}",
                if has_streaming { "✅ YES" } else { "❌ NO" }
            );
        }
        if let Some(input) = model.input_token_limit {
            println!("  Input tokens: {}", input);
        }
        if let Some(output) = model.output_token_limit {
            println!("  Output tokens: {}", output);
        }
    } else {
        println!("\n⚠️  Gemini 3 Pro Preview not found in model list");
        println!("  Model may not be available yet or requires different API access");
    }

    if let Some(next_token) = response.next_page_token {
        println!("\nMore models available (next page token: {next_token})");
    }

    Ok(())
}
