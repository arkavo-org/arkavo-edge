#![allow(clippy::disallowed_methods)]

use arkavo_gemini::RestClient;
use base64::{Engine as _, engine::general_purpose};

fn get_api_key() -> String {
    std::env::var("GEMINI_API_KEY").unwrap_or_else(|_| "test_key".to_string())
}

fn get_model() -> String {
    std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "models/gemini-3-pro-preview".to_string())
}

fn should_skip_integration_tests() -> bool {
    std::env::var("GEMINI_API_KEY").is_err() || get_api_key() == "test_key"
}

fn create_test_image_base64() -> String {
    let png_data = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    general_purpose::STANDARD.encode(&png_data)
}

#[tokio::test]
async fn test_image_input_base64() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let api_key = get_api_key();
    let model = get_model();
    let client = RestClient::new(&api_key, &model);

    let image_data = create_test_image_base64();

    let prompt = format!(
        r#"What do you see in this image? This is a test image.
Image data: data:image/png;base64,{image_data}"#
    );

    let mut stream = client
        .stream_generate_content(&prompt, None, None)
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
                eprintln!("Stream error: {e:?}");
                break;
            }
        }
    }

    assert!(had_response, "Should receive a response for image input");

    let response_lower = response_text.to_lowercase();
    let has_visual_keywords = response_lower.contains("image")
        || response_lower.contains("picture")
        || response_lower.contains("visual")
        || response_lower.contains("see")
        || response_lower.contains("pixel");

    let has_text_only_error = response_lower.contains("text only")
        || response_lower.contains("cannot process image")
        || response_lower.contains("unsupported");

    assert!(
        !has_text_only_error,
        "Model should not return 'text only' error: {response_text}"
    );

    if !has_visual_keywords {
        eprintln!("Warning: Response may not acknowledge visual content: {response_text}");
    }
}

#[tokio::test]
async fn test_vision_language_integration() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let api_key = get_api_key();
    let model = get_model();
    let client = RestClient::new(&api_key, &model);

    let image_data = create_test_image_base64();

    let prompt = format!(
        r#"Describe this image in detail and then answer: What color is dominant?
Image: data:image/png;base64,{image_data}"#
    );

    let mut stream = client
        .stream_generate_content(&prompt, None, None)
        .await
        .expect("Failed to create stream");

    let mut response_text = String::new();

    while let Some(result) = stream.next().await {
        if let Ok(chunk) = result
            && let Some(text) = chunk.text
        {
            response_text.push_str(&text);
        }
    }

    assert!(
        !response_text.is_empty(),
        "Should receive non-empty response"
    );

    assert!(
        response_text.len() > 20,
        "Response should be substantial (>20 chars), got: {}",
        response_text.len()
    );
}

#[tokio::test]
async fn test_multimodal_error_handling() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let api_key = get_api_key();
    let model = get_model();
    let client = RestClient::new(&api_key, &model);

    let invalid_image_data = "not_a_valid_base64_image";

    let prompt = format!(
        r#"What's in this image?
Image: data:image/png;base64,{invalid_image_data}"#
    );

    let result = client.stream_generate_content(&prompt, None, None).await;

    if let Ok(mut stream) = result {
        let mut had_error = false;
        let mut response = String::new();

        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    if let Some(text) = chunk.text {
                        response.push_str(&text);
                    }
                }
                Err(_) => {
                    had_error = true;
                    break;
                }
            }
        }

        assert!(
            had_error || !response.is_empty(),
            "Should either error or provide response for invalid image"
        );
    }
}

#[tokio::test]
async fn test_image_and_text_combined() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let api_key = get_api_key();
    let model = get_model();
    let client = RestClient::new(&api_key, &model);

    let image_data = create_test_image_base64();

    let prompt = format!(
        r#"Here's an image and a question:
Image: data:image/png;base64,{image_data}

Question: What is 2+2? Also, can you see the image I provided?"#
    );

    let mut stream = client
        .stream_generate_content(&prompt, None, None)
        .await
        .expect("Failed to create stream");

    let mut response_text = String::new();

    while let Some(result) = stream.next().await {
        if let Ok(chunk) = result
            && let Some(text) = chunk.text
        {
            response_text.push_str(&text);
        }
    }

    let response_lower = response_text.to_lowercase();

    let has_math_answer = response_lower.contains('4') || response_lower.contains("four");

    assert!(
        has_math_answer,
        "Should answer the math question in combined input: {response_text}"
    );

    assert!(
        response_text.len() > 10,
        "Should provide substantial response"
    );
}
