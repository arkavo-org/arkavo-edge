use anyhow::Result;
use arkavo_llm::{LlamaCppProvider, Message, Provider, Role};
use arkavo_ui_generator::vision::{ModelSize, Qwen25VLModelLoader};
use base64::Engine;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    println!("Qwen2.5-VL Vision Test");
    println!("======================\n");

    let model_size = if env::args().any(|arg| arg == "--3b") {
        ModelSize::Small3B
    } else {
        ModelSize::Medium7B
    };

    let loader = Qwen25VLModelLoader::new()?;
    let paths = loader.ensure_model(model_size).await?;

    println!("\nLoading Qwen2.5-VL model with vision support...");
    let provider = LlamaCppProvider::new_with_mmproj(
        "qwen25vl".to_string(),
        paths.model.to_string_lossy().to_string(),
        paths.mmproj.to_string_lossy().to_string(),
    )?;

    println!("✓ Model loaded successfully!");

    let test_image = env::args()
        .skip(1)
        .find(|arg| !arg.starts_with("--"))
        .and_then(|img_path| {
            println!("Using image: {}", img_path);
            let image_data = std::fs::read(&img_path).ok()?;
            let base64_image = base64::engine::general_purpose::STANDARD.encode(&image_data);
            Some(base64_image)
        });

    if test_image.is_none() {
        println!("No image provided - testing text-only mode");
    }

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
