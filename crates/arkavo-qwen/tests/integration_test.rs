use arkavo_qwen::{
    Message as QwenMessage, MessageRole, Provider, QwenClient, QwenConfig, QwenProvider, QwenRegion,
};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_complete_basic() {
    let mock_server = MockServer::start().await;

    let response_body = serde_json::json!({
        "id": "chatcmpl-123",
        "object": "chat.completion",
        "created": 1234567890_u64,
        "model": "qwen3-32b",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Hello! How can I help you today?"
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 20,
            "total_tokens": 30
        }
    });

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer test_api_key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .mount(&mock_server)
        .await;

    let config = QwenConfig {
        api_key: "test_api_key".to_string(),
        base_url: mock_server.uri(),
        model: "qwen3-32b".to_string(),
        region: QwenRegion::International,
        ..Default::default()
    };

    let provider = QwenProvider::new(config).unwrap();

    let messages = vec![QwenMessage {
        role: MessageRole::User,
        content: "Hello".to_string(),
        images: None,
    }];

    let response = provider.complete(messages).await.unwrap();
    assert!(response.contains("Hello!"));
}

#[tokio::test]
async fn test_authentication_error() {
    let mock_server = MockServer::start().await;

    let error_response = serde_json::json!({
        "error": {
            "message": "Invalid API key",
            "type": "invalid_request_error"
        }
    });

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(error_response))
        .mount(&mock_server)
        .await;

    let config = QwenConfig {
        api_key: "invalid_key".to_string(),
        base_url: mock_server.uri(),
        model: "qwen3-32b".to_string(),
        region: QwenRegion::International,
        ..Default::default()
    };

    let provider = QwenProvider::new(config).unwrap();

    let messages = vec![QwenMessage {
        role: MessageRole::User,
        content: "Hello".to_string(),
        images: None,
    }];

    let result = provider.complete(messages).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_model_not_found_error() {
    let mock_server = MockServer::start().await;

    let error_response = serde_json::json!({
        "error": {
            "message": "Model not found: qwen3-999b",
            "type": "invalid_request_error"
        }
    });

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(404).set_body_json(error_response))
        .mount(&mock_server)
        .await;

    let config = QwenConfig {
        api_key: "test_key".to_string(),
        base_url: mock_server.uri(),
        model: "qwen3-999b".to_string(),
        region: QwenRegion::International,
        ..Default::default()
    };

    let provider = QwenProvider::new(config).unwrap();

    let messages = vec![QwenMessage {
        role: MessageRole::User,
        content: "Hello".to_string(),
        images: None,
    }];

    let result = provider.complete(messages).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_streaming_basic() {
    let mock_server = MockServer::start().await;

    let stream_data = "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"qwen3-32b\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"qwen3-32b\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" world\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n";

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(stream_data))
        .mount(&mock_server)
        .await;

    let config = QwenConfig {
        api_key: "test_key".to_string(),
        base_url: mock_server.uri(),
        model: "qwen3-32b".to_string(),
        region: QwenRegion::International,
        ..Default::default()
    };

    let provider = QwenProvider::new(config).unwrap();

    let messages = vec![QwenMessage {
        role: MessageRole::User,
        content: "Hello".to_string(),
        images: None,
    }];

    use futures::StreamExt;
    let mut stream = provider.stream(messages).await.unwrap();

    let mut responses = Vec::new();
    while let Some(result) = stream.next().await {
        responses.push(result.unwrap());
    }

    assert!(responses.len() >= 2);
    assert_eq!(responses[0].content, "Hello");
    assert_eq!(responses[1].content, " world");
}

#[tokio::test]
async fn test_client_builder() {
    let config = QwenConfig::default();
    let client = QwenClient::new(config).unwrap();

    let client = client
        .with_model("qwen3-14b")
        .with_temperature(0.8)
        .with_top_p(0.9)
        .with_max_tokens(2048);

    assert_eq!(client.config().model, "qwen3-14b");
    assert_eq!(client.config().temperature, Some(0.8));
    assert_eq!(client.config().top_p, Some(0.9));
    assert_eq!(client.config().max_tokens, Some(2048));
}

#[tokio::test]
async fn test_region_configuration() {
    let intl_config = QwenConfig {
        api_key: "test_key".to_string(),
        region: QwenRegion::International,
        ..Default::default()
    };

    assert_eq!(
        intl_config.base_url,
        "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"
    );

    let cn_config = QwenConfig {
        api_key: "test_key".to_string(),
        base_url: QwenRegion::China.base_url().to_string(),
        region: QwenRegion::China,
        ..Default::default()
    };

    assert_eq!(
        cn_config.base_url,
        "https://dashscope.aliyuncs.com/compatible-mode/v1"
    );
}
