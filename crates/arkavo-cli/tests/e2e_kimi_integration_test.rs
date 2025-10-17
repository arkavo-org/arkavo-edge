#![allow(clippy::disallowed_methods)]
#![allow(clippy::future_not_send)]
#![allow(dead_code)]
#![allow(clippy::format_push_string)]
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::unnecessary_debug_formatting)]
#![allow(clippy::lines_filter_map_ok)]
#![allow(clippy::manual_strip)]
#![allow(clippy::needless_continue)]
#![allow(unused_imports)]
#![allow(clippy::zombie_processes)]
#![allow(clippy::unreadable_literal)]
#![allow(clippy::ignore_without_reason)]
#![allow(clippy::unnecessary_unwrap)]
#![allow(unreachable_pub)]

mod e2e_test_infrastructure;

use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires MOONSHOT_API_KEY"]
async fn test_kimi_api_direct() -> Result<(), Box<dyn std::error::Error>> {
    // Skip if API key not set
    let api_key = match std::env::var("MOONSHOT_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            eprintln!("MOONSHOT_API_KEY not set, skipping direct API test");
            return Ok(());
        }
    };

    // Test direct KIMI API using the client
    use arkavo_kimi::provider::{Message, Role};
    use arkavo_kimi::{KimiClient, KimiConfig, Model};

    let config = KimiConfig {
        api_key,
        model: Model::MoonshotV1_128k, // Use 128k model for K2-like performance
        ..Default::default()
    };

    let client = KimiClient::new(config)?;

    let messages = vec![
        Message {
            role: Role::System,
            content: "You are a helpful AI assistant testing the KIMI K2 integration.".to_string(),
            images: None,
        },
        Message {
            role: Role::User,
            content: "What is 2 + 2? Reply with just the number.".to_string(),
            images: None,
        },
    ];

    let response = client
        .create_chat_completion(messages, None, None, None, None)
        .await?;

    println!("KIMI K2 response: {response}");
    assert!(response.contains('4'), "Response should contain '4'");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires MOONSHOT_API_KEY"]
async fn test_kimi_streaming() -> Result<(), Box<dyn std::error::Error>> {
    // Skip if API key not set
    let api_key = match std::env::var("MOONSHOT_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            eprintln!("MOONSHOT_API_KEY not set, skipping streaming test");
            return Ok(());
        }
    };

    use arkavo_kimi::provider::{Message, Role};
    use arkavo_kimi::{KimiClient, KimiConfig, Model};

    let config = KimiConfig {
        api_key,
        model: Model::MoonshotV1_128k,
        ..Default::default()
    };

    let client = KimiClient::new(config)?;

    let messages = vec![Message {
        role: Role::User,
        content: "Count from 1 to 5.".to_string(),
        images: None,
    }];

    use futures::StreamExt;

    let mut stream = client
        .create_chat_completion_stream(messages, None, None, None, None)
        .await?;

    let mut full_response = String::new();
    while let Some(result) = stream.next().await {
        match result {
            Ok(response) => {
                print!("{}", response.content);
                full_response.push_str(&response.content);
            }
            Err(e) => {
                eprintln!("Stream error: {e}");
                break;
            }
        }
    }
    println!(); // New line after streaming

    assert!(
        !full_response.is_empty(),
        "Should receive streamed response"
    );
    assert!(
        full_response.contains('1') && full_response.contains('5'),
        "Response should contain numbers 1 through 5"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires MOONSHOT_API_KEY"]
async fn test_kimi_long_context() -> Result<(), Box<dyn std::error::Error>> {
    // Skip if API key not set
    let api_key = match std::env::var("MOONSHOT_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            eprintln!("MOONSHOT_API_KEY not set, skipping long context test");
            return Ok(());
        }
    };

    use arkavo_kimi::provider::{Message, Role};
    use arkavo_kimi::{KimiClient, KimiConfig, Model};

    let config = KimiConfig {
        api_key,
        model: Model::MoonshotV1_128k,     // 128k context window
        timeout: Duration::from_secs(120), // Longer timeout for long context
        ..Default::default()
    };

    let client = KimiClient::new(config)?;

    // Create a long context by repeating a pattern
    let long_text = "This is a test sentence. ".repeat(1000); // ~25k chars
    println!("Long text length: {} chars", long_text.len());
    println!(
        "Expected 'test' count: {}",
        long_text.matches("test").count()
    );

    let messages = vec![Message {
        role: Role::User,
        content: format!(
            "Here is a long text:\n{long_text}\n\nHow many times does the word 'test' appear?"
        ),
        images: None,
    }];

    let response = client
        .create_chat_completion(messages, None, None, None, None)
        .await?;

    println!("Long context response: {response}");

    // The response should contain "1000" as that's the actual count
    assert!(
        response.contains("1000")
            || response.contains("thousand")
            || (response.contains("test")
                && (response.contains("times") || response.contains("occurrences"))),
        "Response should indicate the word 'test' appears in the text"
    );

    Ok(())
}
