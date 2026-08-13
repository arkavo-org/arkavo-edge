//! Live e2e against xAI **Responses API** surfaces not covered by the
//! router/`LlmClient` path in `e2e_grok.rs`:
//! - non-streaming `create` captures `response_id` / `last_response_id`
//! - SSE streaming ends with a single `done: true`
//!
//! ```sh
//! export XAI_API_KEY=<key>
//! cargo test -p arkavo-llm --test e2e_xai_responses -- --ignored --nocapture
//! ```

#![allow(clippy::disallowed_methods)]

use arkavo_llm::providers::xai_responses::{ResponsesConfig, ResponsesProvider};
use arkavo_llm::{Message, Provider};

fn provider() -> Option<ResponsesProvider> {
    let api_key = std::env::var("XAI_API_KEY").ok()?;
    let base_url =
        std::env::var("XAI_BASE_URL").unwrap_or_else(|_| "https://api.x.ai/v1".to_string());
    Some(
        ResponsesProvider::new(ResponsesConfig::for_agent(
            api_key,
            base_url,
            "grok-4.6".to_string(),
        ))
        .expect("ResponsesProvider construction"),
    )
}

fn is_transient(err: &impl std::fmt::Display) -> bool {
    let s = err.to_string().to_lowercase();
    s.contains("429")
        || s.contains("rate limit")
        || s.contains("quota")
        || s.contains("timeout")
        || s.contains("timed out")
        || s.contains("insufficient")
}

#[tokio::test]
#[ignore = "Requires XAI_API_KEY — live Responses API create + response id"]
async fn responses_create_records_response_id() {
    let Some(p) = provider() else {
        eprintln!("XAI_API_KEY not set — skip");
        return;
    };

    match p
        .create(
            vec![Message::user(
                "Reply with exactly the word pong and nothing else.",
            )],
            None,
            Some(64),
            None,
        )
        .await
    {
        Ok(result) => {
            assert!(
                result.content.to_lowercase().contains("pong"),
                "got: {}",
                result.content
            );
            assert!(!result.response_id.is_empty(), "response id required");
            assert_eq!(
                p.last_response_id().as_deref(),
                Some(result.response_id.as_str())
            );
            // finish_reason is Responses status (e.g. "completed"), not OpenAI tool_calls.
            if let Some(status) = &result.finish_reason {
                assert!(
                    !status.is_empty(),
                    "status should be non-empty when present"
                );
            }
        }
        Err(err) if is_transient(&err) => {
            eprintln!("skip transient: {err}");
        }
        Err(err) => panic!("Responses create failed: {err}"),
    }
}

#[tokio::test]
#[ignore = "Requires XAI_API_KEY — live Responses stream"]
async fn responses_streams_text_deltas() {
    use futures::StreamExt;

    let Some(p) = provider() else {
        eprintln!("XAI_API_KEY not set — skip");
        return;
    };

    let stream = match p.stream(vec![Message::user("Say hi in one word.")]).await {
        Ok(s) => s,
        Err(err) if is_transient(&err) => {
            eprintln!("skip transient: {err}");
            return;
        }
        Err(err) => panic!("stream open failed: {err}"),
    };

    let mut content = String::new();
    let mut done_count = 0usize;
    let mut stream = stream;
    while let Some(item) = stream.next().await {
        match item {
            Ok(chunk) => {
                content.push_str(&chunk.content);
                if chunk.done {
                    done_count += 1;
                }
            }
            Err(err) if is_transient(&err) => {
                eprintln!("skip transient mid-stream: {err}");
                return;
            }
            Err(err) => panic!("stream error: {err}"),
        }
    }
    assert_eq!(done_count, 1, "stream must end with exactly one done");
    assert!(!content.is_empty(), "expected streamed text");
}

#[test]
fn provider_defaults_to_low_effort_and_ephemeral_store() {
    let cfg = ResponsesConfig::default();
    assert_eq!(
        cfg.reasoning_effort,
        arkavo_llm::providers::xai_responses::ReasoningEffort::Low
    );
    assert_eq!(cfg.model, "grok-4.6");
    assert!(!cfg.store, "default store is false for agent privacy");
}
