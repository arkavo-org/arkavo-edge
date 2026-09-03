//! The seam a completion is inspected through before it is streamed
//! (SENT-007, SENT-009).
//!
//! A completion cannot be unstreamed. Once a token reaches the consumer every
//! later decision about it is a decision about something that has already left,
//! so inspection has to sit *between* production and release rather than
//! alongside it. This module is that seam: [`gated`] turns a provider stream
//! into one whose chunks have each been through a [`ReleaseGate`].
//!
//! The gate itself is not here. This crate is underneath the classifier in the
//! dependency graph, so the trait is defined here and implemented by the wiring
//! crate above both — which also means a build with no classifier links none of
//! it and streams exactly as before.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::Stream;

use crate::stream::StreamResponse;
use crate::{Error, Result};

/// What a gate decided about the text it has been given so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    /// Text cleared for the consumer. May be empty while the gate accumulates.
    Release(String),
    /// A label fired. Nothing further is released from this completion.
    Blocked,
}

/// Inspects generated text before it is released.
///
/// Implementations hold their own buffer, so a gate can withhold text across
/// several chunks and release it once the window covering it has been seen.
pub trait ReleaseGate: Send + Sync {
    /// Accept produced text and return whatever is now cleared.
    fn admit(&self, chunk: &str) -> GateOutcome;

    /// The completion is over: inspect and return whatever is still held.
    fn finish(&self) -> GateOutcome;

    /// The consumer went away. Held text is discarded rather than flushed —
    /// a disconnect is not an inspection.
    fn discard(&self);
}

/// Message a consumer receives when a gate blocks a completion.
///
/// Uniform and uninformative on purpose (SENT-011): a message that named the
/// label or the position would let a caller bisect the content it could not
/// see by watching where generation stopped.
pub const GATE_BLOCKED: &str = "response withheld by data policy";

/// Wrap a provider stream so nothing reaches the consumer uninspected.
pub fn gated<S>(inner: S, gate: Arc<dyn ReleaseGate>) -> GatedStream<S>
where
    S: Stream<Item = Result<StreamResponse>> + Send + Unpin,
{
    GatedStream {
        inner,
        gate,
        blocked: false,
        finished: false,
    }
}

pub struct GatedStream<S> {
    inner: S,
    gate: Arc<dyn ReleaseGate>,
    blocked: bool,
    finished: bool,
}

impl<S> Stream for GatedStream<S>
where
    S: Stream<Item = Result<StreamResponse>> + Send + Unpin,
{
    type Item = Result<StreamResponse>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.finished {
            return Poll::Ready(None);
        }
        loop {
            match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    // The inner stream ended without a done marker. The tail is
                    // still inspected rather than dropped or flushed.
                    this.finished = true;
                    return Poll::Ready(Some(finish(&this.gate, &mut this.blocked, None)));
                }
                Poll::Ready(Some(Err(e))) => {
                    // The provider stopped mid-completion. The gate outlives
                    // this stream, so text it admitted but has not yet windowed
                    // would otherwise sit in its buffer and be prefixed onto
                    // the next completion — the same cross-completion
                    // contamination the abandoned-consumer path discards for.
                    // `discard` and not `finish`: an error is not an
                    // inspection, and there is no consumer left to release to.
                    this.gate.discard();
                    this.finished = true;
                    return Poll::Ready(Some(Err(e)));
                }
                Poll::Ready(Some(Ok(chunk))) => {
                    if this.blocked {
                        // Nothing more leaves a blocked completion, but the
                        // inner stream is still drained so the provider is not
                        // left with a half-consumed generation.
                        if chunk.done {
                            this.finished = true;
                            return Poll::Ready(None);
                        }
                        continue;
                    }
                    if chunk.done {
                        this.finished = true;
                        // The final chunk can carry text: a provider that sends
                        // its last tokens and the done marker in one message
                        // would otherwise have that text bypass the gate
                        // entirely, which is the one span most likely to hold
                        // the end of a completion.
                        //
                        // What the admit clears is kept, not just tested for a
                        // block. A gate releases by the window, so a whole
                        // completion arriving as one done chunk — which is
                        // every non-streaming provider — clears its full
                        // windows here and only its tail at `finish`. Reading
                        // the admit for `Blocked` alone dropped everything but
                        // that tail, and truncated any answer longer than one
                        // window rather than withholding it.
                        let mut released = String::new();
                        if !chunk.content.is_empty() {
                            match this.gate.admit(&chunk.content) {
                                GateOutcome::Blocked => {
                                    this.blocked = true;
                                    return Poll::Ready(Some(Err(Error::Provider(
                                        GATE_BLOCKED.to_string(),
                                    ))));
                                }
                                GateOutcome::Release(text) => released = text,
                            }
                        }
                        return Poll::Ready(Some(
                            finish(&this.gate, &mut this.blocked, Some(chunk)).map(|mut last| {
                                released.push_str(&last.content);
                                last.content = released;
                                last
                            }),
                        ));
                    }
                    match this.gate.admit(&chunk.content) {
                        GateOutcome::Blocked => {
                            this.blocked = true;
                            this.finished = true;
                            return Poll::Ready(Some(Err(Error::Provider(
                                GATE_BLOCKED.to_string(),
                            ))));
                        }
                        GateOutcome::Release(text) => {
                            // An empty release is the gate still accumulating.
                            // Yielding it would emit a chunk carrying nothing,
                            // so the loop asks the provider for more instead.
                            if text.is_empty() {
                                continue;
                            }
                            return Poll::Ready(Some(Ok(StreamResponse {
                                content: text,
                                ..chunk
                            })));
                        }
                    }
                }
            }
        }
    }
}

impl<S> Drop for GatedStream<S> {
    /// A consumer that walks away leaves held text behind, and a gate outlives
    /// the completion it was inspecting — it is the session's, not this
    /// stream's. Telling it the completion is over is what keeps the abandoned
    /// text from being carried into the next one. `discard` and not `finish`:
    /// a disconnect is not an inspection, so the text is dropped rather than
    /// judged and released to nobody.
    fn drop(&mut self) {
        if !self.finished {
            self.gate.discard();
        }
    }
}

/// Inspect and emit the tail. `done` carries the final chunk's metadata when
/// the provider sent one, so timing information is not lost to the gate.
fn finish(
    gate: &Arc<dyn ReleaseGate>,
    blocked: &mut bool,
    done: Option<StreamResponse>,
) -> Result<StreamResponse> {
    match gate.finish() {
        GateOutcome::Blocked => {
            *blocked = true;
            Err(Error::Provider(GATE_BLOCKED.to_string()))
        }
        GateOutcome::Release(text) => {
            let mut final_chunk = done.unwrap_or(StreamResponse {
                content: String::new(),
                reasoning_content: None,
                done: true,
                inference_timing: None,
            });
            final_chunk.content = text;
            final_chunk.done = true;
            Ok(final_chunk)
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use std::sync::Mutex;

    /// A gate that releases whole chunks, blocking on any chunk that completes
    /// a canary. Enough to exercise the combinator without a classifier.
    struct Canary {
        seen: Mutex<String>,
        needle: String,
        window: usize,
    }

    impl Canary {
        fn new(needle: &str, window: usize) -> Arc<Self> {
            Arc::new(Self {
                seen: Mutex::new(String::new()),
                needle: needle.to_string(),
                window,
            })
        }

        fn judge(&self, flush: bool) -> GateOutcome {
            // The guard is scoped so it drops before the outcome is returned,
            // which is what the caller's next `admit` needs.
            {
                let mut seen = self.seen.lock().expect("lock");
                if seen.contains(&self.needle) {
                    seen.clear();
                    GateOutcome::Blocked
                } else if !flush && seen.len() < self.window {
                    GateOutcome::Release(String::new())
                } else {
                    GateOutcome::Release(std::mem::take(&mut seen))
                }
            }
        }
    }

    impl ReleaseGate for Canary {
        fn admit(&self, chunk: &str) -> GateOutcome {
            self.seen.lock().expect("lock").push_str(chunk);
            self.judge(false)
        }

        fn finish(&self) -> GateOutcome {
            self.judge(true)
        }

        fn discard(&self) {
            self.seen.lock().expect("lock").clear();
        }
    }

    fn chunks(parts: &[&str]) -> Vec<Result<StreamResponse>> {
        let mut out: Vec<Result<StreamResponse>> = parts
            .iter()
            .map(|p| {
                Ok(StreamResponse {
                    content: (*p).to_string(),
                    reasoning_content: None,
                    done: false,
                    inference_timing: None,
                })
            })
            .collect();
        out.push(Ok(StreamResponse {
            content: String::new(),
            reasoning_content: None,
            done: true,
            inference_timing: None,
        }));
        out
    }

    async fn drain(parts: &[&str], gate: Arc<dyn ReleaseGate>) -> (String, bool) {
        let inner = futures::stream::iter(chunks(parts));
        let mut stream = gated(Box::pin(inner), gate);
        let mut seen = String::new();
        let mut errored = false;
        while let Some(item) = stream.next().await {
            match item {
                Ok(chunk) => seen.push_str(&chunk.content),
                Err(_) => {
                    errored = true;
                    break;
                }
            }
        }
        (seen, errored)
    }

    /// SENT-007: a completion with nothing in it arrives whole and in order.
    #[tokio::test]
    async fn a_clean_completion_passes_through_unchanged() {
        let (seen, errored) = drain(
            &["the quick ", "brown fox ", "jumps over"],
            Canary::new("CANARY", 8),
        )
        .await;

        assert!(!errored);
        assert_eq!(seen, "the quick brown fox jumps over");
    }

    /// SENT-007: a window that fires is never released, and nothing after it
    /// reaches the consumer either.
    #[tokio::test]
    async fn a_blocked_completion_releases_nothing_after_the_label() {
        let (seen, errored) = drain(
            &["harmless ", "then CANARY ", "and the rest"],
            Canary::new("CANARY", 8),
        )
        .await;

        assert!(errored, "the consumer must be told the stream ended");
        assert!(!seen.contains("CANARY"));
        assert!(!seen.contains("the rest"));
    }

    /// The final chunk can carry text. Admitting it is what stops the last
    /// tokens of a completion from bypassing the gate.
    #[tokio::test]
    async fn text_arriving_on_the_done_chunk_is_still_inspected() {
        let inner = futures::stream::iter(vec![
            Ok(StreamResponse {
                content: "a clean opening ".to_string(),
                reasoning_content: None,
                done: false,
                inference_timing: None,
            }),
            Ok(StreamResponse {
                content: "and then CANARY at the very end".to_string(),
                reasoning_content: None,
                done: true,
                inference_timing: None,
            }),
        ]);
        let mut stream = gated(Box::pin(inner), Canary::new("CANARY", 4096));

        let mut seen = String::new();
        let mut refused = false;
        while let Some(item) = stream.next().await {
            match item {
                Ok(chunk) => seen.push_str(&chunk.content),
                Err(_) => {
                    refused = true;
                    break;
                }
            }
        }

        assert!(refused, "the done chunk's text must not bypass the gate");
        assert!(!seen.contains("CANARY"), "{seen}");
    }

    /// And clean text on the done chunk is still delivered.
    #[tokio::test]
    async fn clean_text_on_the_done_chunk_still_reaches_the_consumer() {
        let inner = futures::stream::iter(vec![
            Ok(StreamResponse {
                content: "an opening ".to_string(),
                reasoning_content: None,
                done: false,
                inference_timing: None,
            }),
            Ok(StreamResponse {
                content: "and a closing".to_string(),
                reasoning_content: None,
                done: true,
                inference_timing: None,
            }),
        ]);
        let mut stream = gated(Box::pin(inner), Canary::new("CANARY", 4096));

        let mut seen = String::new();
        while let Some(item) = stream.next().await {
            seen.push_str(&item.expect("clean text is not refused").content);
        }

        assert_eq!(seen, "an opening and a closing");
    }

    /// A gate that releases in whole windows and holds the remainder until the
    /// completion ends, which is the shape a classifier-backed gate has: it
    /// judges a window at a time and cannot judge a partial one.
    struct Windowed {
        held: Mutex<String>,
        window: usize,
    }

    impl Windowed {
        fn new(window: usize) -> Arc<Self> {
            Arc::new(Self {
                held: Mutex::new(String::new()),
                window,
            })
        }
    }

    impl ReleaseGate for Windowed {
        fn admit(&self, chunk: &str) -> GateOutcome {
            let mut held = self.held.lock().expect("lock");
            held.push_str(chunk);
            let split = held.len() - held.len() % self.window;
            let rest = held.split_off(split);
            GateOutcome::Release(std::mem::replace(&mut *held, rest))
        }

        fn finish(&self) -> GateOutcome {
            GateOutcome::Release(std::mem::take(&mut *self.held.lock().expect("lock")))
        }

        fn discard(&self) {
            self.held.lock().expect("lock").clear();
        }
    }

    /// A completion that arrives as one done chunk — every non-streaming
    /// provider, and every `RouteStream::from_response` — reaches the consumer
    /// whole. The done arm used to read its admit for a block and nothing else,
    /// so every window the gate cleared there was dropped and the caller got
    /// only the tail: a silent truncation of any answer longer than one window,
    /// and only with a gate set.
    #[tokio::test]
    async fn a_whole_completion_on_the_done_chunk_is_not_truncated() {
        // Four full windows and a partial one, so the answer is split across
        // both halves of the gate's contract.
        let answer = "abcdefghij".repeat(7);
        let inner = futures::stream::iter(vec![Ok(StreamResponse {
            content: answer.clone(),
            reasoning_content: None,
            done: true,
            inference_timing: None,
        })]);
        let mut stream = gated(Box::pin(inner), Windowed::new(16));

        let mut seen = String::new();
        while let Some(item) = stream.next().await {
            seen.push_str(&item.expect("clean text is not refused").content);
        }

        assert_eq!(seen, answer);
    }

    /// SENT-007 edge case: the completion ends mid-window and the tail is still
    /// inspected before release.
    #[tokio::test]
    async fn a_tail_shorter_than_a_window_is_still_inspected() {
        let (seen, errored) = drain(&["short tail with CANARY"], Canary::new("CANARY", 4096)).await;

        assert!(errored);
        assert!(seen.is_empty(), "the tail must not be flushed uninspected");
    }

    /// A gate that counts the discards it is told about, so a test can assert
    /// on a notification rather than on the buffer it clears.
    struct Counting {
        discards: Mutex<usize>,
    }

    impl Counting {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                discards: Mutex::new(0),
            })
        }

        fn discards(&self) -> usize {
            *self.discards.lock().expect("lock")
        }
    }

    impl ReleaseGate for Counting {
        // Releases each chunk whole, so the consumer gets a yield per chunk and
        // can stop between two of them.
        fn admit(&self, chunk: &str) -> GateOutcome {
            GateOutcome::Release(chunk.to_string())
        }

        fn finish(&self) -> GateOutcome {
            GateOutcome::Release(String::new())
        }

        fn discard(&self) {
            *self.discards.lock().expect("lock") += 1;
        }
    }

    /// SENT-007 edge case: the consumer goes away mid-completion. The gate
    /// outlives the stream, so the text this one was holding has to be dropped
    /// rather than left for the next completion to inherit.
    #[tokio::test]
    async fn dropping_a_partly_consumed_stream_discards_what_it_held() {
        let gate = Counting::new();
        {
            let inner = futures::stream::iter(chunks(&["first ", "second ", "third"]));
            let mut stream = gated(Box::pin(inner), gate.clone());
            stream.next().await;
            assert_eq!(gate.discards(), 0, "not while the stream is alive");
        }

        assert_eq!(gate.discards(), 1);
    }

    /// A provider that errors mid-completion ends it as surely as a consumer
    /// walking away does, and the gate has to be told: text it admitted but has
    /// not windowed would otherwise be inherited by the next completion.
    #[tokio::test]
    async fn a_provider_error_mid_stream_discards_what_the_gate_held() {
        let gate = Counting::new();
        {
            let inner = futures::stream::iter(vec![
                Ok(StreamResponse {
                    content: "held".to_string(),
                    reasoning_content: None,
                    done: false,
                    inference_timing: None,
                }),
                Err(Error::Provider("the provider gave up".to_string())),
            ]);
            let mut stream = gated(Box::pin(inner), gate.clone());

            stream.next().await.expect("a chunk").expect("clean text");
            let error = stream.next().await.expect("the error");
            assert!(error.is_err());
            assert_eq!(gate.discards(), 1, "told as soon as the error is seen");
        }

        // And dropping the stream afterwards does not discard a second time:
        // the completion is already over.
        assert_eq!(gate.discards(), 1);
    }

    /// And a completion that ran to its end has nothing to discard: finishing
    /// already inspected the tail, and a discard here would tell a session-wide
    /// gate to throw away a completion that was properly closed.
    #[tokio::test]
    async fn dropping_a_finished_stream_discards_nothing() {
        let gate = Counting::new();
        {
            let inner = futures::stream::iter(chunks(&["all of it"]));
            let mut stream = gated(Box::pin(inner), gate.clone());
            while stream.next().await.is_some() {}
        }

        assert_eq!(gate.discards(), 0);
    }

    /// The final chunk keeps the provider's metadata, so gating a stream does
    /// not cost the timing the caller measures models with.
    #[tokio::test]
    async fn the_final_chunk_is_still_marked_done() {
        let inner = futures::stream::iter(chunks(&["all clear"]));
        let mut stream = gated(Box::pin(inner), Canary::new("CANARY", 4096));

        let mut last_done = false;
        while let Some(Ok(chunk)) = stream.next().await {
            last_done = chunk.done;
        }

        assert!(last_done);
    }
}
