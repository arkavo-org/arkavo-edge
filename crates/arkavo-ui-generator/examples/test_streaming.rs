#![allow(clippy::disallowed_methods)]

use arkavo_router::Router;
use arkavo_ui_generator::streaming::StreamingGenerator;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _api_key = std::env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY required");

    println!("Creating router...");
    let router = Arc::new(Router::new().await?);
    let generator = StreamingGenerator::new(router)?;

    println!("Starting component generation...\n");
    let mut rx = generator
        .generate_part(
            "Counter Button",
            "An increment button for a counter",
            "Build a simple counter UI",
        )
        .await?;

    while let Some(chunk) = rx.recv().await {
        println!("Chunk type: {:?}", chunk.chunk_type);
        println!("Content length: {}", chunk.content.len());
        println!("Done: {}", chunk.done);

        if !chunk.content.is_empty() {
            println!(
                "Content preview: {}",
                &chunk.content[..chunk.content.len().min(100)]
            );
        }
        println!("---");

        if chunk.done {
            println!("\n✓ Generation complete!");
            break;
        }
    }

    Ok(())
}
