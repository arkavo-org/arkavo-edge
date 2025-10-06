use arkavo_gemini::LiveSessionClient;
use std::env;
use tokio::time::{Duration, sleep};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let api_key = env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY required");
    let model = "models/gemini-2.0-flash-exp";

    println!("Creating client (no tools)...");
    let client = LiveSessionClient::new(api_key, model);

    println!("Connecting...");
    client.connect().await?;
    println!("✓ Connected!");

    sleep(Duration::from_millis(500)).await;

    println!("\nSending simple prompt...");
    client.send_prompt("Say hello in one word").await?;

    println!("Waiting 10 seconds for response...");
    sleep(Duration::from_secs(10)).await;

    client.close().await?;
    println!("Done!");

    Ok(())
}
