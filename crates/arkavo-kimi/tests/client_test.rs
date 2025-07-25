use arkavo_kimi::provider::Message;
use arkavo_kimi::{KimiClient, KimiConfig, Model};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_chat_completion_success() {
    let mock_server = MockServer::start().await;

    let response_body = r#"{
        "id": "chatcmpl-123",
        "object": "chat.completion",
        "created": 1234567890,
        "model": "moonshot-v1-8k",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Paris is the capital of France."
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 8,
            "total_tokens": 18
        }
    }"#;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("Authorization", "Bearer test-api-key"))
        .respond_with(ResponseTemplate::new(200).set_body_string(response_body))
        .mount(&mock_server)
        .await;

    let config = KimiConfig {
        api_key: "test-api-key".to_string(),
        base_url: mock_server.uri(),
        model: Model::MoonshotV1_8k,
        ..Default::default()
    };

    let client = KimiClient::new(config).unwrap();
    let messages = vec![Message {
        role: arkavo_kimi::provider::Role::User,
        content: "What is the capital of France?".to_string(),
        images: None,
    }];

    let response = client
        .create_chat_completion(messages, None, None, None, None)
        .await
        .unwrap();

    assert_eq!(response, "Paris is the capital of France.");
}

#[tokio::test]
async fn test_rate_limit_retry() {
    let mock_server = MockServer::start().await;

    // First request returns rate limit error
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("Authorization", "Bearer test-api-key"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "1")
                .set_body_string(r#"{"error": {"message": "Rate limit exceeded"}}"#),
        )
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    // Second request succeeds
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("Authorization", "Bearer test-api-key"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{
                "id": "chatcmpl-123",
                "object": "chat.completion",
                "created": 1234567890,
                "model": "moonshot-v1-8k",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Success after retry"
                    },
                    "finish_reason": "stop"
                }]
            }"#,
        ))
        .mount(&mock_server)
        .await;

    let mut config = KimiConfig {
        api_key: "test-api-key".to_string(),
        base_url: mock_server.uri(),
        model: Model::MoonshotV1_8k,
        ..Default::default()
    };
    config.retry_config.initial_delay = std::time::Duration::from_millis(100);
    config.retry_config.max_delay = std::time::Duration::from_secs(2);

    let client = KimiClient::new(config).unwrap();
    let messages = vec![Message {
        role: arkavo_kimi::provider::Role::User,
        content: "Test message".to_string(),
        images: None,
    }];

    let response = client
        .create_chat_completion(messages, None, None, None, None)
        .await
        .unwrap();

    assert_eq!(response, "Success after retry");
}

#[tokio::test]
async fn test_authentication_failure() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_string(r#"{"error": {"message": "Invalid API key"}}"#),
        )
        .mount(&mock_server)
        .await;

    let config = KimiConfig {
        api_key: "invalid-key".to_string(),
        base_url: mock_server.uri(),
        model: Model::MoonshotV1_8k,
        ..Default::default()
    };

    let client = KimiClient::new(config).unwrap();
    let messages = vec![Message {
        role: arkavo_kimi::provider::Role::User,
        content: "Test".to_string(),
        images: None,
    }];

    let result = client
        .create_chat_completion(messages, None, None, None, None)
        .await;

    assert!(result.is_err());
    let error = result.unwrap_err();
    assert!(matches!(
        error,
        arkavo_kimi::KimiError::AuthenticationFailed { .. }
    ));
}
