use anyhow::Result;
use arkavo_llm::{LlamaCppProvider, Message, Provider, Role};
use base64::Engine;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let model_path = "/Volumes/SSD/huggingface/hub/models--NexaAI--Qwen3-VL-4B-Instruct-GGUF/snapshots/cbfbb80d8f8a5ffaaa404f1fab632b1e7c3bb0e8/Qwen3-VL-4B-Instruct.Q4_0.gguf";
    let mmproj_path = "/Volumes/SSD/huggingface/hub/models--NexaAI--Qwen3-VL-4B-Instruct-GGUF/snapshots/cbfbb80d8f8a5ffaaa404f1fab632b1e7c3bb0e8/mmproj.F16.gguf";

    println!("Loading Qwen3-VL model with vision support...");
    println!("Model: {}", model_path);
    println!("mmproj: {}", mmproj_path);

    let provider = LlamaCppProvider::new_with_mmproj(
        "qwen3vl".to_string(),
        model_path.to_string(),
        mmproj_path.to_string(),
    )?;

    println!("✓ Model loaded successfully!");

    let test_image = if let Some(img_path) = env::args().nth(1) {
        println!("Using image: {}", img_path);
        let image_data = std::fs::read(&img_path)?;
        let base64_image = base64::engine::general_purpose::STANDARD.encode(&image_data);
        Some(base64_image)
    } else {
        println!("No image provided - testing text-only mode");
        None
    };

    let message = if let Some(img) = test_image {
        Message {
            role: Role::User,
            content: "Describe what you see in this image in detail.".to_string(),
            images: Some(vec![img]),
        }
    } else {
        Message {
            role: Role::User,
            content: "What is 2+2?".to_string(),
            images: None,
        }
    };

    println!("\nSending message to model...");
    let response = provider
        .complete_with_options(vec![message], Some(200))
        .await?;

    println!("\n=== Response ===");
    println!("{}", response);
    println!("================\n");

    Ok(())
}
