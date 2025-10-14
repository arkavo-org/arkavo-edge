//! SSE stress tests for arkavo-gemini
//!
//! These tests use #[tokio::test] which internally uses Runtime::block_on.
#![allow(clippy::disallowed_methods)]
#![allow(clippy::never_loop)] // Some loops break immediately for testing

use arkavo_gemini::types::StreamChunk;
use bytes::Bytes;
use futures::stream;
use std::pin::Pin;
use tokio_stream::Stream;

type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>;

fn create_test_stream(chunks: Vec<&str>) -> ByteStream {
    let byte_chunks: Vec<Result<Bytes, reqwest::Error>> = chunks
        .into_iter()
        .map(|s| Ok(Bytes::from(s.to_string())))
        .collect();

    Box::pin(stream::iter(byte_chunks))
}

#[tokio::test]
async fn test_split_json_across_chunks() {
    let chunk1 = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello";
    let chunk2 = " World\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}]}\n\n";

    let test_stream = create_test_stream(vec![chunk1, chunk2]);
    let mut sse_stream = arkavo_gemini::sse_stream::GeminiSseStream::new(test_stream);

    let mut received_text = String::new();
    while let Some(result) = sse_stream.next().await {
        match result {
            Ok(response) => {
                if let Some(text) = response.text {
                    received_text.push_str(&text);
                }
                if response.done {
                    break;
                }
            }
            Err(e) => panic!("Stream error: {e}"),
        }
    }

    assert_eq!(received_text, "Hello World");
}

#[tokio::test]
async fn test_multiline_data_field() {
    let sse_data = "data: {\"candidates\":[{\"content\":\n\
                    data: {\"parts\":[{\"text\":\"Multi-line\"}],\n\
                    data: \"role\":\"model\"},\"finishReason\":\"STOP\"}]}\n\n";

    let test_stream = create_test_stream(vec![sse_data]);
    let mut sse_stream = arkavo_gemini::sse_stream::GeminiSseStream::new(test_stream);

    let mut received_text = String::new();
    while let Some(result) = sse_stream.next().await {
        match result {
            Ok(response) => {
                if let Some(text) = response.text {
                    received_text.push_str(&text);
                }
                if response.done {
                    break;
                }
            }
            Err(e) => panic!("Stream error: {e}"),
        }
    }

    assert_eq!(received_text, "Multi-line");
}

#[tokio::test]
async fn test_large_response() {
    let large_text = "x".repeat(50000);
    let json = format!(
        "data: {{\"candidates\":[{{\"content\":{{\"parts\":[{{\"text\":\"{large_text}\"}}],\"role\":\"model\"}},\"finishReason\":\"STOP\"}}]}}\n\n"
    );

    let test_stream = create_test_stream(vec![&json]);
    let mut sse_stream = arkavo_gemini::sse_stream::GeminiSseStream::new(test_stream);

    let mut received_text = String::new();
    while let Some(result) = sse_stream.next().await {
        match result {
            Ok(response) => {
                if let Some(text) = response.text {
                    received_text.push_str(&text);
                }
                if response.done {
                    break;
                }
            }
            Err(e) => panic!("Stream error: {e}"),
        }
    }

    assert_eq!(received_text.len(), 50000);
}

#[tokio::test]
async fn test_malformed_json_salvage() {
    // Realistic scenario: stream starts OK, then gets malformed mid-way
    let chunk1 = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Good\"}],\"role\":\"model\"}}]}\n\n";
    let chunk2 = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\" data\"}],\"role\":\"model\"}}]}\n\n";
    let chunk3 = "data: {invalid json that breaks parsing\n\n";

    let test_stream = create_test_stream(vec![chunk1, chunk2, chunk3]);
    let mut sse_stream = arkavo_gemini::sse_stream::GeminiSseStream::new(test_stream);

    let mut received_text = String::new();

    while let Some(result) = sse_stream.next().await {
        match result {
            Ok(response) => {
                if let Some(text) = response.text {
                    received_text.push_str(&text);
                }
                if response.done {
                    break;
                }
            }
            Err(_) => {
                break;
            }
        }
    }

    // Should salvage the accumulated "Good data" when hitting malformed JSON
    assert_eq!(received_text, "Good data");
}

#[tokio::test]
async fn test_empty_candidates() {
    let json = "data: {\"candidates\":[]}\n\n";

    let test_stream = create_test_stream(vec![json]);
    let mut sse_stream = arkavo_gemini::sse_stream::GeminiSseStream::new(test_stream);

    let mut response_count = 0;
    while let Some(result) = sse_stream.next().await {
        match result {
            Ok(response) => {
                response_count += 1;
                if response.done {
                    break;
                }
            }
            Err(e) => panic!("Should not error on empty candidates: {e}"),
        }
    }

    assert!(response_count >= 1);
}

#[tokio::test]
async fn test_missing_optional_fields() {
    let json = "data: {\"candidates\":[{\"content\":{\"parts\":[]}}]}\n\n";

    let test_stream = create_test_stream(vec![json]);
    let mut sse_stream = arkavo_gemini::sse_stream::GeminiSseStream::new(test_stream);

    let mut got_response = false;
    while let Some(result) = sse_stream.next().await {
        match result {
            Ok(_response) => {
                got_response = true;
                break;
            }
            Err(e) => panic!("Should not error on missing optional fields: {e}"),
        }
    }

    assert!(got_response);
}

#[test]
fn test_streamchunk_deserialize_with_defaults() {
    let json = r#"{"candidates":[]}"#;
    let chunk: StreamChunk = serde_json::from_str(json).unwrap();
    assert_eq!(chunk.candidates.len(), 0);

    let json2 = r#"{"candidates":[{"content":{"parts":[]}}]}"#;
    let chunk2: StreamChunk = serde_json::from_str(json2).unwrap();
    assert_eq!(chunk2.candidates.len(), 1);
    assert_eq!(chunk2.candidates[0].finish_reason, None);
    assert_eq!(chunk2.candidates[0].content.role, "");
}
