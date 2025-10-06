use arkavo_gemini::LiveSessionClient;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let api_key = env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY required");
    let model = "models/gemini-2.0-flash-exp";

    println!("Creating client...");
    let client = LiveSessionClient::new(api_key, model);

    println!("Connecting...");
    client.connect().await?;
    println!("✓ Connected!");

    println!("\nWaiting 5 seconds to see any messages...");
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

    println!("\nSending simple prompt...");
    client.send_prompt("Hello").await?;

    println!("Waiting 5 more seconds...");
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

    client.close().await?;
    println!("Done!");

    Ok(())
}
