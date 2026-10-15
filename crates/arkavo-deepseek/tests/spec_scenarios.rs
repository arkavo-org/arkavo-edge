//! Spec-scenario tests for the DeepSeek provider (deepseek.spec.yaml)
//!
//! These tests use #[tokio::test] which internally uses Runtime::block_on.
//! This is the correct pattern for async tests, so we allow the clippy lint.
#![allow(clippy::disallowed_methods)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use arkavo_deepseek::{
    ChatMessage, DeepSeekClient, DeepSeekConfig, DeepSeekError, MessageContent, Role, Tool,
    ToolFunction,
};
use arkavo_test_macros::spec;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ok_completion_body(content: &str) -> Value {
    json!({
        "id": "chatcmpl_1",
        "object": "chat.completion",
        "created": 1_234_567_890,
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 5,
            "completion_tokens": 3,
            "total_tokens": 8
        }
    })
}

fn user_message(text: &str) -> ChatMessage {
    ChatMessage {
        role: Role::User,
        content: MessageContent::Text {
            content: text.to_string(),
        },
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }
}

fn test_config(base_url: String) -> DeepSeekConfig {
    DeepSeekConfig {
        api_key: "test-api-key".to_string(),
        base_url,
        model: "deepseek-chat".to_string(),
        ..Default::default()
    }
}

/// DEEP-001: the request is serialized to the OpenAI-compatible schema and
/// POSTed with bearer auth; the response is parsed into a ChatMessage.
#[spec("DEEP-001")]
#[tokio::test]
async fn test_chat_completion_request_serialized_and_posted() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_completion_body("Pong")))
        .mount(&mock_server)
        .await;

    let config = DeepSeekConfig {
        max_tokens: Some(128),
        temperature: Some(0.2),
        ..test_config(mock_server.uri())
    };
    let client = DeepSeekClient::new(config).unwrap();

    let response = client
        .complete(vec![user_message("Ping")], None, None, None)
        .await
        .unwrap();
    assert_eq!(response.choices[0].message.content.as_deref(), Some("Pong"));

    let requests = mock_server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "expected exactly one HTTP POST");
    let request = &requests[0];

    let auth = request
        .headers
        .get("authorization")
        .expect("authorization header required for all requests");
    assert_eq!(auth, "Bearer test-api-key");

    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(body["model"], "deepseek-chat");
    assert_eq!(body["stream"], false);
    assert_eq!(body["max_tokens"], 128);
    assert_eq!(body["temperature"], 0.2);
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"], "Ping");
}

/// DEEP-003: a reasoning model response carries the chain-of-thought in
/// `reasoning_content` alongside the extracted final answer in `content`.
#[spec("DEEP-003")]
#[tokio::test]
async fn test_reasoning_model_returns_reasoning_content_and_answer() {
    let mock_server = MockServer::start().await;

    let response_body = json!({
        "id": "chatcmpl_r1",
        "object": "chat.completion",
        "created": 1_234_567_890,
        "model": "deepseek-reasoner",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "reasoning_content": "Let me work through this step by step: 15 + 27 = 42.",
                "content": "The answer is 42"
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 12,
            "completion_tokens": 30,
            "total_tokens": 42
        }
    });

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
        .mount(&mock_server)
        .await;

    let config = DeepSeekConfig {
        model: "deepseek-reasoner".to_string(),
        ..test_config(mock_server.uri())
    };
    let client = DeepSeekClient::new(config).unwrap();

    let response = client
        .complete(vec![user_message("What is 15 + 27?")], None, None, None)
        .await
        .unwrap();

    let message = &response.choices[0].message;
    let reasoning = message
        .reasoning_content
        .as_deref()
        .expect("reasoning model response must expose chain-of-thought");
    assert!(reasoning.contains("step by step"));
    assert_eq!(message.content.as_deref(), Some("The answer is 42"));
}

/// DEEP-004: a 429 is categorized as rate-limited and retried using the
/// server's retry-after hint; the retried request succeeds.
#[spec("DEEP-004")]
#[tokio::test]
async fn test_rate_limit_error_retried_then_succeeds() {
    let mock_server = MockServer::start().await;

    let attempts = Arc::new(AtomicU32::new(0));
    let attempts_in_responder = attempts.clone();

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(move |_req: &wiremock::Request| {
            let attempt = attempts_in_responder.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                ResponseTemplate::new(429)
                    .append_header("retry-after", "0")
                    .set_body_json(json!({
                        "error": {
                            "message": "Rate limit exceeded",
                            "type": "rate_limit_exceeded",
                            "code": "rate_limit_exceeded"
                        }
                    }))
            } else {
                ResponseTemplate::new(200).set_body_json(ok_completion_body("Recovered"))
            }
        })
        .mount(&mock_server)
        .await;

    let config = DeepSeekConfig {
        max_retries: 3,
        ..test_config(mock_server.uri())
    };
    let client = DeepSeekClient::new(config).unwrap();

    let response = client
        .complete(vec![user_message("Hello")], None, None, None)
        .await
        .unwrap();

    assert_eq!(
        response.choices[0].message.content.as_deref(),
        Some("Recovered")
    );
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        2,
        "expected the initial request plus one retry"
    );
}

/// DEEP-004: retrying stops at max_retries and the final categorized
/// RateLimited error is returned to the caller.
#[spec("DEEP-004")]
#[tokio::test]
async fn test_max_retries_enforced_then_rate_limit_error_returned() {
    let mock_server = MockServer::start().await;

    let attempts = Arc::new(AtomicU32::new(0));
    let attempts_in_responder = attempts.clone();

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(move |_req: &wiremock::Request| {
            attempts_in_responder.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(429)
                .append_header("retry-after", "0")
                .set_body_json(json!({
                    "error": {
                        "message": "Rate limit exceeded",
                        "type": "rate_limit_exceeded",
                        "code": "rate_limit_exceeded"
                    }
                }))
        })
        .mount(&mock_server)
        .await;

    let config = DeepSeekConfig {
        max_retries: 2,
        ..test_config(mock_server.uri())
    };
    let client = DeepSeekClient::new(config).unwrap();

    let result = client
        .complete(vec![user_message("Hello")], None, None, None)
        .await;

    match result {
        Err(DeepSeekError::RateLimited { retry_after, .. }) => {
            assert_eq!(
                retry_after,
                Some(std::time::Duration::from_secs(0)),
                "retry-after hint from the server must be preserved"
            );
        }
        other => panic!("Expected RateLimited error after retries exhausted, got: {other:?}"),
    }
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        3,
        "expected the initial request plus max_retries=2 retries"
    );
}

/// DEEP-004: a 401 is categorized as an authentication failure, which is not
/// retryable, so the error is returned immediately without further attempts.
#[spec("DEEP-004")]
#[tokio::test]
async fn test_authentication_error_categorized_and_not_retried() {
    let mock_server = MockServer::start().await;

    let attempts = Arc::new(AtomicU32::new(0));
    let attempts_in_responder = attempts.clone();

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(move |_req: &wiremock::Request| {
            attempts_in_responder.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(401).set_body_json(json!({
                "error": {
                    "message": "Invalid API key provided",
                    "type": "authentication_error",
                    "code": "invalid_api_key"
                }
            }))
        })
        .mount(&mock_server)
        .await;

    let config = DeepSeekConfig {
        max_retries: 3,
        ..test_config(mock_server.uri())
    };
    let client = DeepSeekClient::new(config).unwrap();

    let result = client
        .complete(vec![user_message("Hello")], None, None, None)
        .await;

    match result {
        Err(DeepSeekError::AuthenticationFailed { message }) => {
            assert!(message.contains("Invalid API key"));
        }
        other => panic!("Expected AuthenticationFailed error, got: {other:?}"),
    }
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        1,
        "authentication failures must not be retried"
    );
}

/// DEEP-005: in strict mode the tool schema is sent to the API with
/// strict:true, the returned JSON arguments validate against the schema, and
/// malformed arguments surface a parse error.
#[spec("DEEP-005")]
#[tokio::test]
async fn test_strict_mode_schema_sent_and_json_validated() {
    let mock_server = MockServer::start().await;

    let schema = json!({
        "type": "object",
        "properties": {
            "city": {"type": "string"},
            "units": {"type": "string", "enum": ["celsius", "fahrenheit"]}
        },
        "required": ["city", "units"],
        "additionalProperties": false
    });

    let response_body = json!({
        "id": "chatcmpl_strict",
        "object": "chat.completion",
        "created": 1_234_567_890,
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": "{\"city\":\"Paris\",\"units\":\"celsius\"}"
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
        use_strict_mode: true,
        ..test_config(mock_server.uri())
    };
    let client = DeepSeekClient::new(config).unwrap();

    let tool = Tool {
        tool_type: "function".to_string(),
        function: ToolFunction {
            name: "get_weather".to_string(),
            description: "Get weather for a city".to_string(),
            parameters: schema.clone(),
        },
        strict: None,
    };

    let response = client
        .complete(
            vec![user_message("Weather in Paris?")],
            Some(vec![tool]),
            None,
            None,
        )
        .await
        .unwrap();

    let requests = mock_server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["tools"][0]["strict"], true,
        "strict mode must mark tools with strict:true"
    );
    assert_eq!(
        body["tools"][0]["function"]["parameters"], schema,
        "the JSON schema must be sent to the API"
    );

    let tool_calls = response.choices[0].message.tool_calls.as_ref().unwrap();
    let validated = client
        .validate_tool_arguments(&tool_calls[0].function.arguments, &schema, "get_weather")
        .unwrap();
    assert_eq!(validated["city"], "Paris");
    assert_eq!(validated["units"], "celsius");

    let parse_error = client.validate_tool_arguments("{not json", &schema, "get_weather");
    assert!(
        matches!(parse_error, Err(DeepSeekError::ToolArgumentsInvalid { .. })),
        "malformed JSON arguments must produce a parse error"
    );
}
