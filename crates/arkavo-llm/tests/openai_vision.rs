use arkavo_llm::providers::openai::{OpenAIConfig, OpenAIProvider};
use arkavo_llm::{Message, Provider, Role};
use base64::Engine;
use std::fs;
use std::path::Path;

#[path = "common/mod.rs"]
mod common;
use common::ensure_api_key;

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable and GPT-5 access"]
async fn test_gpt4o_vision_basic() {
    let api_key = ensure_api_key();

    let config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-5".to_string(), // GPT-5 supports vision
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config).expect("Failed to create OpenAI provider for GPT-5");

    // Create a simple test image (base64 encoded 1x1 red pixel PNG)
    let test_image_base64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";

    let messages = vec![Message {
        role: Role::User,
        content: "What color is this image? Reply with just the color name.".to_string(),
        images: Some(vec![test_image_base64.to_string()]),
    }];

    let response = provider
        .complete(messages)
        .await
        .expect("Failed to get response from GPT-5 with image");

    println!("GPT-5 vision response: {response}");

    // The test image is a red pixel
    assert!(
        response.to_lowercase().contains("red") || response.to_lowercase().contains("pink"),
        "Response should identify the red color: {response}"
    );
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable and GPT-5 access"]
async fn test_gpt4o_vision_with_text() {
    let api_key = ensure_api_key();

    let config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-5".to_string(),
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config).expect("Failed to create OpenAI provider");

    // Create a test image with text (base64 encoded image with "TEST" text)
    // This is a simplified example - in production you'd generate or load a real image
    let test_image_base64 = create_test_image_with_text();

    let messages = vec![
        Message {
            role: Role::System,
            content: "You are an image analysis assistant.".to_string(),
            images: None,
        },
        Message {
            role: Role::User,
            content: "Describe this image in one sentence.".to_string(),
            images: Some(vec![test_image_base64]),
        },
    ];

    let response = provider
        .complete(messages)
        .await
        .expect("Failed to analyze image with text");

    println!("Image analysis response: {response}");
    assert!(!response.is_empty());
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable and GPT-5 access"]
async fn test_gpt4o_multiple_images() {
    let api_key = ensure_api_key();

    let config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-5".to_string(),
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config).expect("Failed to create OpenAI provider");

    // Create two different colored pixels
    let red_pixel = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";
    let blue_pixel = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPj/HwADBwIAMCbHYQAAAABJRU5ErkJggg==";

    let messages = vec![Message {
        role: Role::User,
        content:
            "I'm showing you two images. Are they the same color? Reply with just 'yes' or 'no'."
                .to_string(),
        images: Some(vec![red_pixel.to_string(), blue_pixel.to_string()]),
    }];

    let response = provider
        .complete(messages)
        .await
        .expect("Failed to compare multiple images");

    println!("Multiple image comparison: {response}");
    assert!(
        response.to_lowercase().contains("no") || response.to_lowercase().contains("different"),
        "Should identify that colors are different: {response}"
    );
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable and GPT-5 access"]
async fn test_gpt4o_vision_streaming() {
    let api_key = ensure_api_key();

    let config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-5".to_string(),
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config).expect("Failed to create OpenAI provider");

    let test_image_base64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";

    let messages = vec![Message {
        role: Role::User,
        content:
            "Describe this image in detail, mentioning its color, size, and any patterns you see."
                .to_string(),
        images: Some(vec![test_image_base64.to_string()]),
    }];

    use futures::StreamExt;
    let mut stream = provider
        .stream(messages)
        .await
        .expect("Failed to create vision stream");

    let mut full_response = String::new();
    let mut chunk_count = 0;

    while let Some(result) = stream.next().await {
        match result {
            Ok(response) => {
                full_response.push_str(&response.content);
                chunk_count += 1;

                if response.done {
                    break;
                }
            }
            Err(e) => {
                panic!("Stream error: {e}");
            }
        }
    }

    println!("Vision streaming response ({chunk_count} chunks): {full_response}");
    assert!(chunk_count > 1, "Should receive multiple chunks");
    assert!(!full_response.is_empty());
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_non_vision_model_with_image_fails() {
    let api_key = ensure_api_key();

    let config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-5".to_string(), // This model doesn't support vision
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config).expect("Failed to create OpenAI provider");

    let test_image_base64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";

    let messages = vec![Message {
        role: Role::User,
        content: "What do you see in this image?".to_string(),
        images: Some(vec![test_image_base64.to_string()]),
    }];

    let result = provider.complete(messages).await;

    // GPT-5 doesn't support images, so this should either fail or ignore the image
    match result {
        Ok(response) => {
            println!("Non-vision model response: {response}");
            // Model might acknowledge it can't see images
            assert!(
                response.to_lowercase().contains("can't")
                    || response.to_lowercase().contains("cannot")
                    || response.to_lowercase().contains("unable")
                    || response.to_lowercase().contains("don't")
                    || response.to_lowercase().contains("text")
            );
        }
        Err(e) => {
            println!("Expected error with non-vision model: {e}");
            assert!(e.to_string().contains("400") || e.to_string().contains("invalid"));
        }
    }
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable and local test image"]
async fn test_gpt4o_with_local_image_file() {
    let api_key = ensure_api_key();

    // Skip if test image doesn't exist
    let test_image_path = Path::new("tests/openai_integration/test_assets/test_image.png");
    if !test_image_path.exists() {
        println!("Skipping test - no test image at {test_image_path:?}");
        return;
    }

    let config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-5".to_string(),
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config).expect("Failed to create OpenAI provider");

    // Read and encode the image
    let image_data = fs::read(test_image_path).expect("Failed to read test image");

    let base64_engine = base64::engine::general_purpose::STANDARD;
    let encoded_image = base64_engine.encode(&image_data);

    let messages = vec![Message {
        role: Role::User,
        content: "What's in this image? Describe it briefly.".to_string(),
        images: Some(vec![encoded_image]),
    }];

    let response = provider
        .complete(messages)
        .await
        .expect("Failed to analyze local image");

    println!("Local image analysis: {response}");
    assert!(!response.is_empty());
}

// Helper function to create a simple test image with text
fn create_test_image_with_text() -> String {
    // This is a simplified base64-encoded PNG with basic shapes
    // In a real test, you might use an image generation library
    "iVBORw0KGgoAAAANSUhEUgAAAAoAAAAKCAYAAACNMs+9AAAAFUlEQVR42mP8/5+hnoEIwDiqkL4KAcT9GO0U4BxoAAAAAElFTkSuQmCC".to_string()
}
