use arkavo_gemini::RestClient;
use std::time::{Duration, Instant};
use tokio::time::timeout;

fn get_api_key() -> String {
    std::env::var("GEMINI_API_KEY").unwrap_or_else(|_| "test_key".to_string())
}

fn get_model() -> String {
    std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "models/gemini-3-pro-preview".to_string())
}

fn should_skip_integration_tests() -> bool {
    std::env::var("GEMINI_API_KEY").is_err() || get_api_key() == "test_key"
}

#[tokio::test]
async fn test_safety_filter_handling() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let api_key = get_api_key();
    let model = get_model();
    let client = RestClient::new(&api_key, &model);

    let safety_boundary_prompt =
        "Tell me about chemical reactions that are considered dangerous in household settings.";

    let mut stream = client
        .stream_generate_content(safety_boundary_prompt, None)
        .await
        .expect("Failed to create stream");

    let mut had_safety_response = false;
    let mut response_text = String::new();

    while let Some(result) = stream.next().await {
        match result {
            Ok(chunk) => {
                if let Some(text) = chunk.text {
                    response_text.push_str(&text);
                    had_safety_response = true;
                }
                if chunk.done {
                    had_safety_response = true;
                }
            }
            Err(e) => {
                let error_msg = format!("{e:?}");
                if error_msg.to_uppercase().contains("SAFETY") {
                    had_safety_response = true;
                }
                break;
            }
        }
    }

    assert!(
        had_safety_response || !response_text.is_empty(),
        "Should handle safety-related queries gracefully"
    );
}

#[tokio::test]
async fn test_rate_limit_backoff() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let api_key = get_api_key();
    let model = get_model();

    let mut tasks = vec![];
    for i in 0..10 {
        let client_clone = RestClient::new(&api_key, &model);
        let task = tokio::spawn(async move {
            let prompt = format!("Quick test {i}");
            let result = client_clone.stream_generate_content(&prompt, None).await;
            result.is_ok()
        });
        tasks.push(task);
    }

    let results: Vec<bool> = futures::future::join_all(tasks)
        .await
        .into_iter()
        .filter_map(|r| r.ok())
        .collect();

    let success_count = results.iter().filter(|&&r| r).count();

    assert!(
        success_count >= 5,
        "At least 50% of concurrent requests should succeed or handle rate limits gracefully"
    );
}

#[tokio::test]
async fn test_context_window_overflow() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let api_key = get_api_key();
    let model = get_model();
    let client = RestClient::new(&api_key, &model);

    let large_text = "Lorem ipsum dolor sit amet. ".repeat(1000);
    let prompt = format!("{large_text}\n\nWhat is the last word in the text above?");

    let mut stream = client
        .stream_generate_content(&prompt, None)
        .await
        .expect("Failed to create stream");

    let mut response_text = String::new();
    let mut had_response = false;

    while let Some(result) = stream.next().await {
        match result {
            Ok(chunk) => {
                if let Some(text) = chunk.text {
                    response_text.push_str(&text);
                    had_response = true;
                }
            }
            Err(e) => {
                eprintln!("Stream error with large context: {e:?}");
                break;
            }
        }
    }

    assert!(had_response, "Should handle large context gracefully");
}

#[tokio::test]
async fn test_token_limit_boundary() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let api_key = get_api_key();
    let model = get_model();
    let client = RestClient::new(&api_key, &model);

    let prompt = "Count from 1 to 10000 with commas between numbers.";

    let result = timeout(Duration::from_secs(30), async {
        let mut stream = client.stream_generate_content(prompt, None).await?;

        let mut token_count = 0;
        let mut had_response = false;

        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    if let Some(text) = chunk.text {
                        token_count += text.split_whitespace().count();
                        had_response = true;
                    }
                }
                Err(_) => break,
            }
        }

        Ok::<_, arkavo_gemini::GeminiError>((had_response, token_count))
    })
    .await;

    match result {
        Ok(Ok((had_response, _token_count))) => {
            assert!(had_response, "Should produce response for large output");
        }
        Ok(Err(e)) => {
            eprintln!("Expected behavior: token limit error {e:?}");
        }
        Err(_) => {
            eprintln!("Timeout is acceptable for token limit boundary test");
        }
    }
}

#[tokio::test]
async fn test_invalid_api_key_handling() {
    let client = RestClient::new("invalid_key_12345", "models/gemini-3-pro-preview");

    let result = client.stream_generate_content("test", None).await;

    assert!(
        result.is_err(),
        "Should error with invalid API key gracefully"
    );
}

#[tokio::test]
async fn test_network_timeout_handling() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let api_key = get_api_key();
    let model = get_model();
    let client = RestClient::new(&api_key, &model);

    let prompt = "Explain quantum computing in detail.";

    let result = timeout(Duration::from_millis(100), async {
        let mut stream = client.stream_generate_content(prompt, None).await?;

        while let Some(_chunk) = stream.next().await {}

        Ok::<_, arkavo_gemini::GeminiError>(())
    })
    .await;

    assert!(result.is_err(), "Should timeout appropriately");
}

#[tokio::test]
async fn test_empty_prompt_handling() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let api_key = get_api_key();
    let model = get_model();
    let client = RestClient::new(&api_key, &model);

    let result = client.stream_generate_content("", None).await;

    if let Ok(mut stream) = result {
        let mut had_response = false;
        while let Some(chunk_result) = stream.next().await {
            if chunk_result.is_ok() {
                had_response = true;
                break;
            }
        }
        assert!(had_response, "Should handle empty prompt");
    }
}

#[tokio::test]
async fn test_very_long_prompt() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let api_key = get_api_key();
    let model = get_model();
    let client = RestClient::new(&api_key, &model);

    let long_prompt = "Tell me a story. ".repeat(5000);

    let result = client.stream_generate_content(&long_prompt, None).await;

    match result {
        Ok(mut stream) => {
            let mut had_response = false;
            while let Some(chunk_result) = stream.next().await {
                if let Ok(chunk) = chunk_result
                    && chunk.text.is_some()
                {
                    had_response = true;
                    break;
                }
            }
            assert!(
                had_response,
                "Should handle very long prompt or error gracefully"
            );
        }
        Err(e) => {
            eprintln!("Expected behavior for very long prompt: {e:?}");
        }
    }
}

#[tokio::test]
async fn test_rapid_sequential_requests() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let api_key = get_api_key();
    let model = get_model();
    let client = RestClient::new(&api_key, &model);

    let start = Instant::now();
    let mut success_count = 0;

    for i in 0..5 {
        let prompt = format!("Quick test {i}");
        let result = timeout(Duration::from_secs(10), async {
            let mut stream = client.stream_generate_content(&prompt, None).await?;
            let mut had_response = false;

            while let Some(chunk_result) = stream.next().await {
                if chunk_result.is_ok() {
                    had_response = true;
                    break;
                }
            }

            Ok::<_, arkavo_gemini::GeminiError>(had_response)
        })
        .await;

        if matches!(result, Ok(Ok(true))) {
            success_count += 1;
        }
    }

    let duration = start.elapsed();

    assert!(
        success_count >= 3,
        "At least 60% of rapid sequential requests should succeed"
    );

    assert!(
        duration.as_secs() < 60,
        "Rapid requests should complete within 60s"
    );
}
