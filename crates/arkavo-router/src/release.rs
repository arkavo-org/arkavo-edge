//! Where a router-produced completion meets the release gate (SENT-007).
//!
//! A completion cannot be unstreamed, so inspection has to sit between the
//! provider and the caller. [`arkavo_llm::gated`] is that seam for a stream;
//! this module adapts it to the two shapes the router actually hands back.
//!
//! Two shapes, because the router has two completion paths. `route` returns a
//! [`RouteStream`], and [`gate_stream`] wraps it. The chat path
//! (`route_chat_spec`) does not stream at all — it awaits a whole
//! `ProviderResponse` and returns it — so [`gate_completion`] drives that text
//! through the same gate in one admit/finish pair. Gating only the stream would
//! leave `arkavo chat`, the caller this exists for, ungated.
//!
//! Nothing here is feature gated: the trait and the combinator live in
//! `arkavo-llm`, which the router already depends on. A router with no gate set
//! never reaches this module.

use std::sync::Arc;

use arkavo_llm::{GATE_BLOCKED, GateOutcome, ReleaseGate, StreamResponse, gated};
use futures::StreamExt;

use crate::stream::{RouteStream, StreamChunk};
use crate::{Error, Result};

/// The refusal a blocked completion becomes.
///
/// SENT-011: uniform and uninformative. It travels as a provider error so that
/// every existing caller already treats it as a failed completion rather than
/// as content.
fn blocked() -> Error {
    Error::Provider(arkavo_llm::Error::Provider(GATE_BLOCKED.to_string()))
}

/// Inspect a whole completion before any of it is returned.
///
/// The gate accumulates and releases in windows, so both halves are needed: the
/// admit covers every full window the text contains and the finish covers the
/// tail that is shorter than one. A block from either discards the text already
/// released here — a completion whose middle was withheld is not a completion,
/// and returning its head would tell the caller where the finding was.
pub fn gate_completion(gate: &Arc<dyn ReleaseGate>, content: &str) -> Result<String> {
    let mut released = match gate.admit(content) {
        GateOutcome::Blocked => return Err(blocked()),
        GateOutcome::Release(text) => text,
    };
    match gate.finish() {
        GateOutcome::Blocked => Err(blocked()),
        GateOutcome::Release(text) => {
            released.push_str(&text);
            Ok(released)
        }
    }
}

/// Wrap a stream so nothing reaches the caller uninspected.
///
/// The two chunk types carry the same two fields the gate reads, so the
/// conversion is total in both directions; the provider metadata `StreamResponse`
/// also carries has no counterpart on a `StreamChunk` and is not invented.
pub fn gate_stream(stream: RouteStream, gate: Arc<dyn ReleaseGate>) -> RouteStream {
    let metadata = stream.metadata().clone();
    // Read before the stream is consumed, and carried onto the rebuilt one:
    // gating a completion is not a decision about its tool calls, and dropping
    // them would change what the caller gets only when a gate is set, which is
    // the one shape of behaviour change nobody would look for.
    let tool_calls = stream.tool_calls().to_vec();
    let inbound = stream.map(|item| match item {
        Ok(chunk) => Ok(StreamResponse {
            content: chunk.content,
            reasoning_content: None,
            done: chunk.done,
            inference_timing: None,
        }),
        // A router error entering the gate is flattened rather than carried:
        // `gated` yields the provider error type, and every `RouteStream` the
        // router builds today comes from a completed response, so no router
        // error actually traverses this seam. Flattening keeps that fact from
        // needing a sidecar channel to stay true.
        Err(e) => Err(arkavo_llm::Error::Provider(e.to_string())),
    });
    let outbound = gated(Box::pin(inbound), gate).map(|item| match item {
        Ok(response) => Ok(StreamChunk {
            content: response.content,
            done: response.done,
        }),
        Err(e) => Err(Error::Provider(e)),
    });
    RouteStream::new(Box::pin(outbound), metadata).with_tool_calls(tool_calls)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::Router;
    use crate::decision::ModelChoice;
    use crate::stream::{RouteMetadata, RouteResponse};
    use std::sync::Mutex;

    /// A gate that records everything it is given and blocks on a needle.
    /// Enough to prove the wiring without a classifier.
    struct Recorder {
        seen: Mutex<String>,
        held: Mutex<String>,
        needle: Option<String>,
    }

    impl Recorder {
        fn new(needle: Option<&str>) -> Arc<Self> {
            Arc::new(Self {
                seen: Mutex::new(String::new()),
                held: Mutex::new(String::new()),
                needle: needle.map(str::to_string),
            })
        }

        fn admitted(&self) -> String {
            self.seen.lock().expect("lock").clone()
        }

        fn judge(&self) -> GateOutcome {
            let mut held = self.held.lock().expect("lock");
            match &self.needle {
                Some(needle) if held.contains(needle.as_str()) => {
                    held.clear();
                    GateOutcome::Blocked
                }
                _ => GateOutcome::Release(std::mem::take(&mut held)),
            }
        }
    }

    impl ReleaseGate for Recorder {
        fn admit(&self, chunk: &str) -> GateOutcome {
            self.seen.lock().expect("lock").push_str(chunk);
            self.held.lock().expect("lock").push_str(chunk);
            self.judge()
        }

        fn finish(&self) -> GateOutcome {
            self.judge()
        }

        fn discard(&self) {
            self.held.lock().expect("lock").clear();
        }
    }

    fn metadata() -> RouteMetadata {
        RouteMetadata {
            model: ModelChoice::LocalGemma270M,
            used_architect_mode: false,
            estimated_cost_usd: 0.0,
        }
    }

    fn stream_of(parts: &[&str]) -> RouteStream {
        let chunks: Vec<Result<StreamChunk>> = parts
            .iter()
            .map(|p| {
                Ok(StreamChunk {
                    content: (*p).to_string(),
                    done: false,
                })
            })
            .collect();
        RouteStream::new(Box::pin(futures::stream::iter(chunks)), metadata())
    }

    async fn drain(stream: RouteStream) -> (String, Option<String>) {
        let mut stream = stream;
        let mut seen = String::new();
        let mut refusal = None;
        while let Some(item) = stream.next().await {
            match item {
                Ok(chunk) => seen.push_str(&chunk.content),
                Err(e) => {
                    refusal = Some(e.to_string());
                    break;
                }
            }
        }
        (seen, refusal)
    }

    /// Every chunk the router produces is offered to the gate, and a gate that
    /// releases everything changes nothing the caller sees.
    #[tokio::test]
    async fn a_gate_sees_every_chunk_of_a_route_stream() {
        let gate = Recorder::new(None);
        let stream = gate_stream(
            stream_of(&["the quick ", "brown fox ", "jumps"]),
            gate.clone(),
        );

        let (seen, refusal) = drain(stream).await;

        assert!(refusal.is_none());
        assert_eq!(seen, "the quick brown fox jumps");
        assert_eq!(gate.admitted(), "the quick brown fox jumps");
    }

    /// A blocking gate ends the stream with the uniform refusal and releases
    /// nothing from the window that fired or after it.
    #[tokio::test]
    async fn a_blocking_gate_ends_the_route_stream_with_gate_blocked() {
        let stream = gate_stream(
            stream_of(&["harmless ", "then CANARY ", "and the rest"]),
            Recorder::new(Some("CANARY")),
        );

        let (seen, refusal) = drain(stream).await;

        let refusal = refusal.expect("the stream must be cut, not completed");
        assert!(refusal.contains(GATE_BLOCKED), "{refusal}");
        assert!(!seen.contains("CANARY"), "{seen}");
        assert!(!seen.contains("the rest"), "{seen}");
    }

    /// The one-shot path is gated by the same gate, since chat never streams.
    #[test]
    fn a_whole_completion_is_inspected_before_it_is_returned() {
        let gate: Arc<dyn ReleaseGate> = Recorder::new(None);

        let released = gate_completion(&gate, "a clean answer").expect("clean text is returned");

        assert_eq!(released, "a clean answer");
    }

    #[test]
    fn a_blocked_completion_returns_the_uniform_refusal() {
        let gate: Arc<dyn ReleaseGate> = Recorder::new(Some("CANARY"));

        let error = gate_completion(&gate, "leading text CANARY trailing")
            .expect_err("a finding must not be returned as content");

        assert!(error.to_string().contains(GATE_BLOCKED), "{error}");
        assert!(!error.to_string().contains("CANARY"), "{error}");
    }

    /// Gating a completion says nothing about its tool calls, and the wrapper
    /// has to carry them: `RouteStream::new` starts with none, so a
    /// `gate_stream` that used it alone made every tool call vanish — but only
    /// when a gate was set.
    #[tokio::test]
    async fn a_gated_stream_still_reports_its_tool_calls() {
        let call = arkavo_llm::ParsedToolCall {
            tool_name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "notes.md"}),
            call_id: Some("call_1".to_string()),
        };
        let stream = gate_stream(
            RouteStream::from_response(RouteResponse {
                content: "reading it now".to_string(),
                tool_calls: vec![call.clone()],
                model: ModelChoice::LocalGemma270M,
                cost_usd: 0.0,
                used_architect_mode: false,
                architect_savings: None,
            }),
            Recorder::new(None),
        );

        let response = stream.complete().await.expect("clean text completes");

        // The whole one-chunk response survives the gate, and so do the tool
        // calls the wrapper was given.
        assert_eq!(response.content, "reading it now");
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].tool_name, call.tool_name);
        assert_eq!(response.tool_calls[0].arguments, call.arguments);
    }

    /// The default is no gate, and a router without one hands the stream back
    /// untouched.
    #[tokio::test]
    async fn a_router_without_a_gate_streams_unchanged() {
        let Ok(router) = Router::new_offline().await else {
            eprintln!("Skipping: Router::new_offline requires llama-cpp");
            return;
        };

        let stream = router.gated_stream(RouteStream::from_response(RouteResponse {
            content: "untouched".to_string(),
            tool_calls: Vec::new(),
            model: ModelChoice::LocalGemma270M,
            cost_usd: 0.0,
            used_architect_mode: false,
            architect_savings: None,
        }));

        let (seen, refusal) = drain(stream).await;

        assert!(refusal.is_none());
        assert_eq!(seen, "untouched");
    }

    /// A gate set on the router is applied to the streams it produces.
    #[tokio::test]
    async fn a_router_with_a_gate_applies_it_to_its_streams() {
        let Ok(mut router) = Router::new_offline().await else {
            eprintln!("Skipping: Router::new_offline requires llama-cpp");
            return;
        };
        router.set_release_gate(Recorder::new(Some("CANARY")));

        let stream = router.gated_stream(RouteStream::from_response(RouteResponse {
            content: "leading CANARY trailing".to_string(),
            tool_calls: Vec::new(),
            model: ModelChoice::LocalGemma270M,
            cost_usd: 0.0,
            used_architect_mode: false,
            architect_savings: None,
        }));

        let (seen, refusal) = drain(stream).await;

        assert!(refusal.is_some_and(|r| r.contains(GATE_BLOCKED)));
        assert!(seen.is_empty(), "{seen}");
    }
}
