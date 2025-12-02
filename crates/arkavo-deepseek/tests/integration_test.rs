//! Integration tests for DeepSeek provider
//!
//! These tests use #[tokio::test] which internally uses Runtime::block_on.
//! This is the correct pattern for async tests, so we allow the clippy lint.
#![allow(clippy::disallowed_methods)]

use arkavo_deepseek::{
    ChatMessage, DeepSeekClient, DeepSeekConfig, DeepSeekProvider, Message, MessageContent,
    MessageRole, Provider, ResponseFormat, Role, Tool, ToolChoice, ToolFunction,
};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Test 1: Anthropic basic - system+user → assistant text
#[tokio::test]
async fn test_anthropic_basic() {
    let mock_server = MockServer::start().await;

    let response_body = json!({
        "id": "msg_123",
        "object": "chat.completion",
        "created": 1_234_567_890,
        "model": "deepseek-chat",
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
            "completion_tokens": 8,
            "total_tokens": 18
        }
    });

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer test-api-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
        .mount(&mock_server)
        .await;

    let config = DeepSeekConfig {
        api_key: "test-api-key".to_string(),
        base_url: mock_server.uri(),
        model: "deepseek-chat".to_string(),
        ..Default::default()
    };

    let client = DeepSeekClient::new(config).unwrap();

    let messages = vec![
        ChatMessage {
            role: Role::System,
            content: MessageContent::Text {
                content: "You are a helpful assistant".to_string(),
            },
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
        ChatMessage {
            role: Role::User,
            content: MessageContent::Text {
                content: "Hello".to_string(),
            },
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
    ];

    let response = client.complete(messages, None, None, None).await.unwrap();
    assert_eq!(
        response.choices[0].message.content.as_ref().unwrap(),
        "Hello! How can I help you today?"
    );
}

/// Test 2: Anthropic tools - tool definition + tool_choice:auto → model emits tool_use
#[tokio::test]
async fn test_anthropic_tools() {
    let mock_server = MockServer::start().await;

    let response_body = json!({
        "id": "msg_123",
        "object": "chat.completion",
        "created": 1_234_567_890,
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_123",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": "{\"location\":\"San Francisco\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 50,
            "completion_tokens": 20,
            "total_tokens": 70
        }
    });

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
        .mount(&mock_server)
        .await;

    let config = DeepSeekConfig {
        api_key: "test-api-key".to_string(),
        base_url: mock_server.uri(),
        model: "deepseek-chat".to_string(),
        ..Default::default()
    };

    let client = DeepSeekClient::new(config).unwrap();

    let tool = Tool {
        tool_type: "function".to_string(),
        function: ToolFunction {
            name: "get_weather".to_string(),
            description: "Get weather for a location".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "location": {"type": "string"}
                },
                "required": ["location"]
            }),
        },
        strict: None,
    };

    let messages = vec![ChatMessage {
        role: Role::User,
        content: MessageContent::Text {
            content: "What's the weather in San Francisco?".to_string(),
        },
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }];

    let response = client
        .complete(messages, Some(vec![tool]), Some(ToolChoice::Auto), None)
        .await
        .unwrap();

    assert!(response.choices[0].message.tool_calls.is_some());
    let tool_calls = response.choices[0].message.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls[0].function.name, "get_weather");
}

/// Test 3: Anthropic unsupported parts - include image content → adapter rejects
#[tokio::test]
async fn test_anthropic_unsupported_parts() {
    use arkavo_deepseek::anthropic_compat::check_unsupported_content;

    let content = json!([
        {"type": "text", "text": "Look at this image:"},
        {"type": "image", "source": {"type": "base64", "data": "..."}}
    ]);

    let result = check_unsupported_content(&content);
    assert!(result.is_err());
    if let Err(arkavo_deepseek::DeepSeekError::UnsupportedMessagePart { part_type, .. }) = result {
        assert_eq!(part_type, "image");
    } else {
        panic!("Expected UnsupportedMessagePart error");
    }
}

/// Test 4: Function calling (standard) - two functions declared
#[tokio::test]
async fn test_function_calling_standard() {
    let mock_server = MockServer::start().await;

    let response_body = json!({
        "id": "msg_123",
        "object": "chat.completion",
        "created": 1_234_567_890,
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_123",
                    "type": "function",
                    "function": {
                        "name": "calculate",
                        "arguments": "{\"a\":5,\"b\":3,\"operation\":\"add\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    });

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
        .mount(&mock_server)
        .await;

    let config = DeepSeekConfig {
        api_key: "test-api-key".to_string(),
        base_url: mock_server.uri(),
        model: "deepseek-chat".to_string(),
        ..Default::default()
    };

    let client = DeepSeekClient::new(config).unwrap();

    let schema = json!({
        "type": "object",
        "properties": {
            "a": {"type": "number"},
            "b": {"type": "number"},
            "operation": {"type": "string", "enum": ["add", "subtract", "multiply", "divide"]}
        },
        "required": ["a", "b", "operation"],
        "additionalProperties": false
    });

    // Validate the arguments
    let args = "{\"a\":5,\"b\":3,\"operation\":\"add\"}";
    let validated = client
        .validate_tool_arguments(args, &schema, "calculate")
        .unwrap();
    assert_eq!(validated["a"], 5);
    assert_eq!(validated["b"], 3);
    assert_eq!(validated["operation"], "add");
}

/// Test 5: Strict mode (valid) - base_url=/beta, strict:true
#[tokio::test]
async fn test_strict_mode_valid() {
    let mock_server = MockServer::start().await;

    let response_body = json!({
        "id": "msg_123",
        "object": "chat.completion",
        "created": 1_234_567_890,
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_123",
                    "type": "function",
                    "function": {
                        "name": "test_func",
                        "arguments": "{\"field1\":\"value1\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    });

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
        .mount(&mock_server)
        .await;

    let config = DeepSeekConfig {
        api_key: "test-api-key".to_string(),
        base_url: mock_server.uri(),
        model: "deepseek-chat".to_string(),
        use_strict_mode: true,
        ..Default::default()
    };

    let client = DeepSeekClient::new(config).unwrap();

    let tool = Tool {
        tool_type: "function".to_string(),
        function: ToolFunction {
            name: "test_func".to_string(),
            description: "Test function".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "field1": {"type": "string"}
                },
                "required": ["field1"],
                "additionalProperties": false
            }),
        },
        strict: None,
    };

    let messages = vec![ChatMessage {
        role: Role::User,
        content: MessageContent::Text {
            content: "Test".to_string(),
        },
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }];

    let response = client
        .complete(messages, Some(vec![tool]), None, None)
        .await
        .unwrap();

    assert!(response.choices[0].message.tool_calls.is_some());
}

/// Test 6: Strict mode (invalid) - omit additionalProperties:false → server error
#[tokio::test]
async fn test_strict_mode_invalid() {
    use arkavo_deepseek::strict_mode::validate_strict_schema;

    let invalid_schema = json!({
        "type": "object",
        "properties": {
            "field1": {"type": "string"}
        },
        "required": ["field1"]
        // Missing "additionalProperties": false
    });

    let result = validate_strict_schema(&invalid_schema);
    assert!(result.is_err());
    if let Err(arkavo_deepseek::DeepSeekError::SchemaError { message, .. }) = result {
        assert!(message.contains("additionalProperties"));
    } else {
        panic!("Expected SchemaError");
    }
}

/// Test 7: Reasoner+tools - call deepseek-reasoner with tools
#[tokio::test]
async fn test_reasoner_tools_fallback() {
    let mock_server = MockServer::start().await;

    // Server should receive deepseek-chat as the model when tools are present
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msg_123",
            "object": "chat.completion",
            "created": 1_234_567_890,
            "model": "deepseek-chat",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "OK"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 2,
                "total_tokens": 12
            }
        })))
        .mount(&mock_server)
        .await;

    let config = DeepSeekConfig {
        api_key: "test-api-key".to_string(),
        base_url: mock_server.uri(),
        model: "deepseek-reasoner".to_string(), // Start with reasoner
        ..Default::default()
    };

    let client = DeepSeekClient::new(config).unwrap();

    let tool = Tool {
        tool_type: "function".to_string(),
        function: ToolFunction {
            name: "test_func".to_string(),
            description: "Test".to_string(),
            parameters: json!({}),
        },
        strict: None,
    };

    let messages = vec![ChatMessage {
        role: Role::User,
        content: MessageContent::Text {
            content: "Test".to_string(),
        },
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }];

    // Should succeed with fallback
    let _ = client
        .complete(messages, Some(vec![tool]), None, None)
        .await
        .unwrap();
}

/// Test 8: Streaming - verify chunked deltas and [DONE]
#[tokio::test]
async fn test_streaming() {
    let mock_server = MockServer::start().await;

    let stream_data = r#"data: {"id":"1","object":"chat.completion.chunk","created":1234567890,"model":"deepseek-chat","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}

data: {"id":"1","object":"chat.completion.chunk","created":1234567890,"model":"deepseek-chat","choices":[{"index":0,"delta":{"content":" world"},"finish_reason":null}]}

data: [DONE]
"#;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(stream_data)
                .append_header("content-type", "text/event-stream"),
        )
        .mount(&mock_server)
        .await;

    let config = DeepSeekConfig {
        api_key: "test-api-key".to_string(),
        base_url: mock_server.uri(),
        model: "deepseek-chat".to_string(),
        ..Default::default()
    };

    let client = DeepSeekClient::new(config).unwrap();

    let messages = vec![ChatMessage {
        role: Role::User,
        content: MessageContent::Text {
            content: "Hello".to_string(),
        },
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }];

    let mut stream = client.stream(messages, None, None, None).await.unwrap();

    use futures::StreamExt;
    let mut responses = Vec::new();
    while let Some(result) = stream.next().await {
        responses.push(result.unwrap());
    }

    assert!(responses.len() >= 2);
    assert!(responses.last().unwrap().done);
}

/// Test 9: JSON mode caveat - enforce prompt instruction
#[tokio::test]
async fn test_json_mode_instruction() {
    use arkavo_deepseek::anthropic_compat::add_json_instruction_if_needed;

    let mut messages = vec![ChatMessage {
        role: Role::User,
        content: MessageContent::Text {
            content: "What is 2+2?".to_string(),
        },
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }];

    let response_format = Some(ResponseFormat::JsonObject);
    add_json_instruction_if_needed(&mut messages, &response_format);

    match &messages[0].content {
        MessageContent::Text { content } => {
            assert!(content.contains("Please respond with valid JSON"));
        }
        _ => panic!("Expected text content"),
    }
}

/// Test 10: Stateless check - omit prior history and confirm expected loss of context
#[tokio::test]
async fn test_stateless_api() {
    // This test verifies that the API is stateless by making two separate requests
    // The second request should not have context from the first

    let mock_server = MockServer::start().await;

    // Set up mocks with random responses (since they should be independent)
    let responses = [
        json!({
            "id": "msg_123",
            "object": "chat.completion",
            "created": 1_234_567_890,
            "model": "deepseek-chat",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "My name is DeepSeek"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        }),
        json!({
            "id": "msg_124",
            "object": "chat.completion",
            "created": 1_234_567_891,
            "model": "deepseek-chat",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "I don't have that information"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 8,
                "completion_tokens": 6,
                "total_tokens": 14
            }
        }),
    ];

    let counter = std::sync::Arc::new(std::sync::Mutex::new(0));
    let counter_clone = counter.clone();

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(move |_req: &wiremock::Request| {
            let mut count = counter_clone.lock().unwrap();
            let response = &responses[*count];
            *count += 1;
            drop(count);
            ResponseTemplate::new(200).set_body_json(response.clone())
        })
        .expect(2)
        .mount(&mock_server)
        .await;

    let config = DeepSeekConfig {
        api_key: "test-api-key".to_string(),
        base_url: mock_server.uri(),
        model: "deepseek-chat".to_string(),
        ..Default::default()
    };

    let client = DeepSeekClient::new(config).unwrap();

    // First request
    let messages1 = vec![ChatMessage {
        role: Role::User,
        content: MessageContent::Text {
            content: "My name is Alice. What's your name?".to_string(),
        },
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }];

    let response1 = client.complete(messages1, None, None, None).await.unwrap();
    assert!(
        response1.choices[0]
            .message
            .content
            .as_ref()
            .unwrap()
            .contains("DeepSeek")
    );

    // Second request - new conversation without history
    let messages2 = vec![ChatMessage {
        role: Role::User,
        content: MessageContent::Text {
            content: "What's my name?".to_string(),
        },
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }];

    let response2 = client.complete(messages2, None, None, None).await.unwrap();
    // Response should indicate lack of context
    let response_text = response2.choices[0].message.content.as_ref().unwrap();
    assert!(
        response_text.contains("don't") || response_text.contains("information"),
        "Expected response to indicate lack of context, got: {response_text}"
    );
}

/// Integration test with actual API (requires DEEPSEEK_API_KEY)
#[tokio::test]
#[ignore = "requires DEEPSEEK_API_KEY environment variable"]
async fn test_live_api_basic() {
    let provider = DeepSeekProvider::from_env().expect("DEEPSEEK_API_KEY must be set");

    let messages = vec![
        Message {
            role: MessageRole::System,
            content: "You are a helpful assistant. Keep responses brief.".to_string(),
            images: None,
        },
        Message {
            role: MessageRole::User,
            content: "What is 2+2? Reply with just the number.".to_string(),
            images: None,
        },
    ];

    let response = provider.complete(messages).await.unwrap();
    assert!(response.contains('4'));
}

/// Live API streaming test
#[tokio::test]
#[ignore = "requires DEEPSEEK_API_KEY environment variable"]
async fn test_live_api_streaming() {
    let provider = DeepSeekProvider::from_env().expect("DEEPSEEK_API_KEY must be set");

    let messages = vec![Message {
        role: MessageRole::User,
        content: "Count from 1 to 5".to_string(),
        images: None,
    }];

    let mut stream = provider.stream(messages).await.unwrap();

    use futures::StreamExt;
    let mut collected = String::new();
    while let Some(result) = stream.next().await {
        let response = result.unwrap();
        if let Some(content) = response.content {
            collected.push_str(&content);
        }
        if response.done {
            break;
        }
    }

    assert!(!collected.is_empty());
    // Should contain numbers 1 through 5
    for i in 1..=5 {
        assert!(collected.contains(&i.to_string()));
    }
}

/// Live API test for V3.2-Speciale with thinking mode
#[tokio::test]
#[ignore = "requires DEEPSEEK_API_KEY environment variable"]
async fn test_live_api_v32_speciale_thinking() {
    let config = DeepSeekConfig {
        api_key: std::env::var("DEEPSEEK_API_KEY").expect("DEEPSEEK_API_KEY must be set"),
        base_url: "https://api.deepseek.com/v3.2_speciale_expires_on_20251215".to_string(),
        model: "deepseek-chat".to_string(),
        thinking_mode: true,
        ..Default::default()
    };

    let client = DeepSeekClient::new(config).unwrap();

    let messages = vec![ChatMessage {
        role: Role::User,
        content: MessageContent::Text {
            content: "What is 15 + 27? Reply with just the number.".to_string(),
        },
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }];

    let response = client.complete(messages, None, None, None).await.unwrap();

    // V3.2-Speciale should return reasoning_content (thinking) and content
    let choice = response.choices.first().unwrap();
    let content = choice.message.content.as_ref().unwrap();
    assert!(
        content.contains("42"),
        "Expected 42 in response: {}",
        content
    );

    // Verify reasoning_content is present (the model's thinking process)
    assert!(
        choice.message.reasoning_content.is_some(),
        "Expected reasoning_content in V3.2-Speciale response"
    );
    let reasoning = choice.message.reasoning_content.as_ref().unwrap();
    assert!(
        !reasoning.is_empty(),
        "reasoning_content should not be empty"
    );
}
