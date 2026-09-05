#![cfg(feature = "llm-remote")]
#![allow(clippy::disallowed_methods)] // Tokio test entrypoints own their runtimes.

use arkavo_llm::providers::{OpenAIResponsesConfig, OpenAIResponsesProvider};
use arkavo_llm::{Message, Provider};
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

async fn read_request(socket: &mut TcpStream) -> Value {
    let mut bytes = Vec::new();
    let (head_end, length) = loop {
        let mut buf = [0; 4096];
        let count = socket.read(&mut buf).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buf[..count]);
        if let Some(end) = bytes.windows(4).position(|p| p == b"\r\n\r\n") {
            let head = std::str::from_utf8(&bytes[..end]).unwrap();
            assert!(head.starts_with("POST /v1/responses HTTP/1.1"));
            let length: usize = head
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            break (end + 4, length);
        }
    };
    while bytes.len() < head_end + length {
        let mut buf = [0; 4096];
        let count = socket.read(&mut buf).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buf[..count]);
    }
    serde_json::from_slice(&bytes[head_end..head_end + length]).unwrap()
}

async fn fixture(
    status: u16,
    body: String,
    sse: bool,
) -> (
    OpenAIResponsesProvider,
    oneshot::Receiver<Value>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let provider = OpenAIResponsesProvider::new(OpenAIResponsesConfig {
        api_key: Some("fixture-token".into()),
        base_url: format!("http://{addr}/v1"),
        ..Default::default()
    })
    .unwrap();
    let (tx, rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        tx.send(read_request(&mut socket).await).unwrap();
        let content_type = if sse {
            "text/event-stream"
        } else {
            "application/json"
        };
        socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
        // Fragment every byte, including the interior of non-ASCII code points.
        for byte in body.as_bytes() {
            if socket.write_all(&[*byte]).await.is_err() {
                break;
            }
        }
    });
    (provider, rx, task)
}

fn completed(text: &str) -> Value {
    json!({"status":"completed", "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}],"usage":{"input_tokens":100,"output_tokens":30,"input_tokens_details":{"cached_tokens":50},"output_tokens_details":{"reasoning_tokens":20}}})
}

#[arkavo_test_macros::spec("ASTRA-002")]
#[tokio::test]
async fn tool_completion_preserves_opaque_state_and_actual_usage() {
    let mut response = completed("");
    response["output"] = json!([
        {"type":"reasoning","id":"rs_1","encrypted_content":"opaque","summary":[]},
        {"type":"function_call","call_id":"call_1","name":"read","arguments":"{\"path\":\"file\"}"}
    ]);
    let (provider, request, task) = fixture(200, response.to_string(), false).await;
    let result = provider
        .complete_with_tools(
            vec![Message::user("read file")],
            Some(json!([{"name":"read","input_schema":{"type":"object"}}])),
            Some(128),
        )
        .await
        .unwrap();
    assert_eq!(result.tool_calls[0].call_id.as_deref(), Some("call_1"));
    assert_eq!(result.response_items[0]["encrypted_content"], "opaque");
    assert!(result.reasoning_content.is_none());
    assert_eq!(result.inference_timing.unwrap().n_eval, 10);
    let request = request.await.unwrap();
    assert_eq!(request["tools"][0]["name"], "read");
    assert_eq!(request["max_output_tokens"], 128);
    assert!(request.get("temperature").is_none());
    task.await.unwrap();
}

#[arkavo_test_macros::spec("ASTRA-001")]
#[tokio::test]
async fn structured_completion_retains_usage() {
    let (provider, request, task) =
        fixture(200, completed("{\"answer\":\"yes\"}").to_string(), false).await;
    let result = provider.complete_with_schema_response(vec![Message::user("answer")], Some(json!({"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"]})), None).await.unwrap();
    assert_eq!(result.content, "{\"answer\":\"yes\"}");
    assert_eq!(
        result.inference_timing.unwrap().n_cached_prompt_eval,
        Some(50)
    );
    assert_eq!(
        request.await.unwrap()["text"]["format"]["type"],
        "json_schema"
    );
    task.await.unwrap();
}

#[arkavo_test_macros::spec("ASTRA-003")]
#[tokio::test]
async fn stream_reassembles_utf8_and_emits_terminal_tail_and_usage_once() {
    let delta = json!({"type":"response.output_text.delta","delta":"🌍"});
    let done = json!({"type":"response.completed","response":completed("🌍 hello")});
    let body = format!("data: {delta}\r\n\r\ndata: {done}\r\n\r\ndata: [DONE]\r\n\r\n");
    let (provider, request, task) = fixture(200, body, true).await;
    let mut stream = provider.stream(vec![Message::user("hi")]).await.unwrap();
    let mut content = String::new();
    let mut done_count = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.unwrap();
        content.push_str(&chunk.content);
        if chunk.done {
            done_count += 1;
            assert_eq!(chunk.inference_timing.unwrap().n_eval, 10);
            assert_eq!(chunk.response_items.len(), 1);
        } else {
            assert!(chunk.response_items.is_empty());
        }
    }
    assert_eq!(content, "🌍 hello");
    assert_eq!(done_count, 1);
    assert_eq!(request.await.unwrap()["stream"], true);
    task.await.unwrap();
}

#[arkavo_test_macros::spec("ASTRA-003")]
#[tokio::test]
async fn http_errors_and_truncated_streams_fail_without_echoing_body() {
    let (provider, request, task) = fixture(401, "sensitive echoed prompt".into(), false).await;
    let error = provider
        .complete(vec![Message::user("test")])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("401"));
    assert!(!error.to_string().contains("sensitive"));
    request.await.unwrap();
    task.await.unwrap();
    let (provider, request, task) = fixture(
        200,
        "data: {\"type\":\"response.created\"}\n\n".into(),
        true,
    )
    .await;
    let mut stream = provider.stream(vec![Message::user("test")]).await.unwrap();
    assert!(stream.next().await.unwrap().is_err());
    assert!(stream.next().await.is_none());
    request.await.unwrap();
    task.await.unwrap();
}

#[arkavo_test_macros::spec("ASTRA-003")]
#[tokio::test]
async fn dropping_stream_closes_the_http_body_without_background_generation() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = OpenAIResponsesProvider::new(OpenAIResponsesConfig {
        api_key: Some("fixture-token".into()),
        base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
        ..Default::default()
    })
    .unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        let data = b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"start\"}\n\n";
        socket
            .write_all(format!("{:x}\r\n", data.len()).as_bytes())
            .await
            .unwrap();
        socket.write_all(data).await.unwrap();
        socket.write_all(b"\r\n").await.unwrap();
        let mut byte = [0];
        let closed =
            tokio::time::timeout(std::time::Duration::from_secs(5), socket.read(&mut byte))
                .await
                .unwrap();
        assert!(matches!(closed, Ok(0) | Err(_)));
    });
    let mut stream = provider.stream(vec![Message::user("test")]).await.unwrap();
    assert_eq!(stream.next().await.unwrap().unwrap().content, "start");
    drop(stream);
    task.await.unwrap();
}

/// Run only with an explicitly supplied OpenAI key. Synthetic prompts, five
/// requests, low effort and a fixed token cap keep the opt-in probe bounded.
#[arkavo_test_macros::spec("ASTRA-003")]
#[tokio::test]
#[ignore = "requires explicitly authorized live OpenAI credentials"]
async fn live_astra_text_tools_schema_and_stream() {
    let provider = OpenAIResponsesProvider::new(OpenAIResponsesConfig {
        reasoning_effort: arkavo_llm::providers::OpenAIReasoningEffort::Low,
        max_output_tokens: 2048,
        ..Default::default()
    })
    .unwrap();
    let first = provider
        .complete_with_tools(
            vec![Message::user("Reply with the single word ready.")],
            None,
            None,
        )
        .await
        .expect("live text completion failed");
    assert!(!first.content.trim().is_empty());
    assert!(
        first
            .inference_timing
            .as_ref()
            .is_some_and(|u| u.n_prompt_eval > 0 && u.n_eval > 0)
    );
    let messages = vec![
        Message::system(
            "Call the provided read_probe tool exactly once before answering. Do not guess its result.",
        ),
        Message::user("Read the probe value, then report it."),
    ];
    let tool = json!([{"type":"function","function":{"name":"read_probe","description":"Return a synthetic probe value.","parameters":{"type":"object","properties":{},"additionalProperties":false}}}]);
    let call = provider
        .complete_with_tools(messages.clone(), Some(tool.clone()), None)
        .await
        .expect("live tool call failed");
    assert_eq!(call.tool_calls.len(), 1);
    assert_eq!(call.tool_calls[0].tool_name, "read_probe");
    let call_id = call.tool_calls[0]
        .call_id
        .clone()
        .expect("missing native call ID");
    assert!(!call.response_items.is_empty());
    let mut continuation = messages;
    continuation.push(call.as_assistant_message());
    continuation.push(Message::tool_result("probe-green", call_id, "read_probe"));
    let answer = provider
        .complete_with_tools(continuation, Some(tool), None)
        .await
        .expect("live continuation failed");
    assert!(answer.tool_calls.is_empty());
    assert!(answer.content.contains("probe-green"));
    let schema = json!({"type":"object","properties":{"ready":{"type":"boolean"}},"required":["ready"],"additionalProperties":false});
    let structured = provider
        .complete_with_schema_response(
            vec![Message::user("Return ready true.")],
            Some(schema),
            None,
        )
        .await
        .expect("live structured completion failed");
    assert_eq!(
        serde_json::from_str::<Value>(&structured.content).unwrap()["ready"],
        true
    );
    assert!(structured.inference_timing.is_some());
    let mut stream = provider
        .stream(vec![Message::user("Reply with the single word ready.")])
        .await
        .expect("live stream setup failed");
    let mut done = false;
    let mut content = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("live stream failed");
        content.push_str(&chunk.content);
        if chunk.done {
            assert!(!done);
            done = true;
            assert!(chunk.inference_timing.is_some());
            assert!(!chunk.response_items.is_empty());
        }
    }
    assert!(done && !content.trim().is_empty());
}

struct DenyPrivate;
impl arkavo_llm::ReleaseGate for DenyPrivate {
    fn admit(&self, text: &str) -> arkavo_llm::GateOutcome {
        if text.contains("private-value") {
            arkavo_llm::GateOutcome::Blocked
        } else {
            arkavo_llm::GateOutcome::Release(text.into())
        }
    }
    fn finish(&self) -> arkavo_llm::GateOutcome {
        arkavo_llm::GateOutcome::Release(String::new())
    }
    fn discard(&self) {}
}
#[async_trait::async_trait]
impl arkavo_llm::ReleaseGateFactory for DenyPrivate {
    fn create(&self, _model: &str) -> std::sync::Arc<dyn arkavo_llm::ReleaseGate> {
        std::sync::Arc::new(Self)
    }
}

#[arkavo_test_macros::spec("ASTRA-006")]
#[tokio::test]
async fn release_gate_inspects_native_tool_arguments_and_retains_denied_usage() {
    let mut response = completed("");
    response["output"] = json!([{"type":"reasoning","encrypted_content":"opaque","summary":[]}, {"type":"function_call","call_id":"call_1","name":"write","arguments":"{\"value\":\"private-value\"}"}]);
    let (provider, request, task) = fixture(200, response.to_string(), false).await;
    let provider =
        arkavo_llm::GuardedProvider::new(Box::new(provider), std::sync::Arc::new(DenyPrivate));
    let error = provider
        .complete_with_tools(vec![Message::user("write")], None, None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains(arkavo_llm::GATE_BLOCKED));
    assert_eq!(error.inference_timing().unwrap().n_prompt_eval, 100);
    request.await.unwrap();
    task.await.unwrap();
}

#[arkavo_test_macros::spec("ASTRA-006")]
#[tokio::test]
async fn schema_response_cannot_bypass_release_policy() {
    let (provider, request, task) =
        fixture(200, completed("private-value").to_string(), false).await;
    let provider =
        arkavo_llm::GuardedProvider::new(Box::new(provider), std::sync::Arc::new(DenyPrivate));
    let error = provider
        .complete_with_schema_response(vec![Message::user("read")], None, None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains(arkavo_llm::GATE_BLOCKED));
    assert_eq!(error.inference_timing().unwrap().n_eval, 10);
    request.await.unwrap();
    task.await.unwrap();
}

#[arkavo_test_macros::spec("ASTRA-001")]
#[tokio::test]
async fn legacy_factory_selects_responses_for_astra() {
    use arkavo_llm::providers::{
        OpenAIProviderFactory, ProviderConfig, ProviderFactory, ProviderType,
    };
    let config = ProviderConfig {
        provider_type: ProviderType::OpenAI,
        base_url: String::new(),
        auth_ref: None,
        default_model: Some("gpt-6-astra".into()),
        timeout_secs: None,
        max_retries: None,
        initial_retry_delay_ms: None,
        backoff_factor: None,
        max_retry_delay_ms: None,
        jitter_factor: None,
        metadata: None,
    };
    OpenAIProviderFactory
        .validate_config(&config)
        .await
        .unwrap();
    match OpenAIProviderFactory.create_provider(&config).await {
        Ok(provider) => {
            assert_eq!(provider.name(), "gpt-6-astra");
            assert!(provider.supports_structured_output());
        }
        Err(error) => assert!(
            error
                .to_string()
                .contains("OPENAI_API_KEY is required for GPT-6 Astra")
        ),
    }
    let mut invalid = config;
    invalid.metadata = Some(std::collections::HashMap::from([(
        "reasoning_effort".into(),
        json!("ultra"),
    )]));
    assert!(
        OpenAIProviderFactory
            .create_provider(&invalid)
            .await
            .is_err()
    );
}

#[arkavo_test_macros::spec("ASTRA-006")]
#[tokio::test]
async fn final_stream_policy_denial_retains_usage_and_withholds_state() {
    let done = json!({"type":"response.completed","response":completed("private-value")});
    let (provider, request, task) = fixture(200, format!("data: {done}\n\n"), true).await;
    let provider =
        arkavo_llm::GuardedProvider::new(Box::new(provider), std::sync::Arc::new(DenyPrivate));
    let mut stream = provider.stream(vec![Message::user("read")]).await.unwrap();
    let error = stream.next().await.unwrap().unwrap_err();
    assert!(error.to_string().contains(arkavo_llm::GATE_BLOCKED));
    assert_eq!(error.inference_timing().unwrap().n_eval, 10);
    assert!(stream.next().await.is_none());
    request.await.unwrap();
    task.await.unwrap();
}
