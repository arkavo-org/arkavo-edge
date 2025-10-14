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
        let base_model = model
            .base_model_id.as_deref()
            .unwrap_or("N/A");
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

    if let Some(next_token) = response.next_page_token {
        println!("\nMore models available (next page token: {next_token})");
    }

    Ok(())
}
