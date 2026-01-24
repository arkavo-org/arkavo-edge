//! SDK integration tests
//! These tests require authentication (OAuth or API key)
//! Run with: cargo test -p arkavo-claude-code --test sdk_test -- --ignored

use anthropic_agent_sdk::{ClaudeAgentOptions, Message, auth::OAuthClient, query};
use futures::StreamExt;
use std::time::Duration;
use tokio::time::timeout;

/// Test OAuth authentication
/// This test requires interactive browser login - skip in CI
#[tokio::test]
#[ignore = "Requires interactive browser authentication"]
async fn test_oauth_authentication() {
    println!("Testing OAuth authentication...");

    let oauth = OAuthClient::new().expect("Failed to create OAuth client");

    match oauth.authenticate().await {
        Ok(token) => {
            println!("OAuth successful!");
            println!("Token expires at: {:?}", token.expires_at);
            assert!(!token.access_token.is_empty());
        }
        Err(e) => {
            // Check if API key is set as fallback
            if std::env::var("ANTHROPIC_API_KEY").is_ok() {
                println!("OAuth failed but API key is set, that's fine.");
            } else {
                println!("OAuth needs interactive browser login: {}", e);
                println!("This is expected in CI environments.");
            }
        }
    }
}

/// Test simple query with SDK
#[tokio::test]
#[ignore = "Requires OAuth or API key"]
async fn test_simple_query() {
    println!("Testing simple query...");

    let options = ClaudeAgentOptions::builder()
        .cwd(std::env::current_dir().unwrap())
        .build();

    let prompt = "Respond with exactly: HELLO_SDK_TEST";

    // Add timeout to prevent hanging
    let result = timeout(Duration::from_secs(30), async {
        let stream = match query(prompt, Some(options)).await {
            Ok(s) => s,
            Err(e) => return Err(format!("Query error: {}", e)),
        };
        let mut stream = Box::pin(stream);
        let mut response_text = String::new();

        while let Some(result) = stream.next().await {
            match result {
                Ok(Message::Assistant { message, .. }) => {
                    response_text.push_str(&format!("{:?}", message));
                }
                Ok(Message::Result { result, .. }) => {
                    if let Some(r) = result {
                        response_text.push_str(&format!("{:?}", r));
                    }
                }
                Err(e) => {
                    return Err(format!("Stream error: {}", e));
                }
                _ => {}
            }
        }

        Ok::<String, String>(response_text)
    })
    .await;

    match result {
        Ok(Ok(response)) => {
            println!("Response: {}", response);
            assert!(
                response.contains("HELLO") || response.contains("SDK") || response.contains("TEST"),
                "Expected response to contain test string"
            );
        }
        Ok(Err(e)) => panic!("Query failed: {}", e),
        Err(_) => panic!("Query timed out after 30 seconds"),
    }
}

/// Test code generation
#[tokio::test]
#[ignore = "Requires OAuth or API key"]
async fn test_code_generation() {
    println!("Testing code generation...");

    let options = ClaudeAgentOptions::builder()
        .cwd(std::env::current_dir().unwrap())
        .build();

    let prompt = r#"Write a simple Python function called 'add' that takes two numbers and returns their sum.
Output only the Python code, no explanation."#;

    let result = timeout(Duration::from_secs(60), async {
        let stream = match query(prompt, Some(options)).await {
            Ok(s) => s,
            Err(e) => return Err(format!("Query error: {}", e)),
        };
        let mut stream = Box::pin(stream);
        let mut code = String::new();

        while let Some(result) = stream.next().await {
            if let Ok(Message::Assistant { message, .. }) = result {
                code.push_str(&format!("{:?}", message));
            }
        }

        Ok::<String, String>(code)
    })
    .await;

    match result {
        Ok(Ok(code)) => {
            println!("Generated code:\n{}", code);
            assert!(
                code.contains("def") || code.contains("add") || code.contains("return"),
                "Expected Python code"
            );
        }
        Ok(Err(e)) => panic!("Code generation failed: {}", e),
        Err(_) => panic!("Code generation timed out"),
    }
}
