//! Streaming holdback (SENT-007, SENT-009).
//!
//! A completion cannot be unstreamed. Once a token has reached the consumer,
//! every later decision about it is a decision about something that has already
//! left, so inspection has to happen *before* release rather than alongside it.
//! That is the threat model, not a nicety.
//!
//! Two properties do the work. Windows overlap, so a label lying across a
//! window boundary is seen whole by at least one inspection — without overlap,
//! splitting a credential across two windows hides it from both. And a window
//! that fires is never released: the buffer has no path that emits held text,
//! because a "release anyway" path is the one an exhausted deadline eventually
//! finds.

use arkavo_protocol::data_classification::SensitivityLevel;

/// Bytes accumulated before a window is offered for inspection.
///
/// A window is what the consumer waits for, so this is latency the user feels.
/// 256 bytes is a sentence or two — long enough that a tier has context to
/// judge, short enough that streaming still reads as streaming.
pub const DEFAULT_WINDOW_BYTES: usize = 256;

/// Bytes of already-released text prepended to each inspection.
///
/// Must exceed the longest span a tier needs to recognize something. The
/// reference tier's five-word shingle is the binding case; 64 bytes covers it
/// with room for a credential that straddles the seam.
pub const DEFAULT_OVERLAP_BYTES: usize = 64;

/// Text to inspect before the part of it that would be released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// The full text to inspect: overlap from already-released output followed
    /// by the candidate. Tiers see this, never the candidate alone.
    pub inspect: String,
    /// Bytes at the end of `inspect` that release would emit.
    pub releasable: usize,
    /// Whether this is the last window of the completion.
    pub final_window: bool,
}

impl Window {
    /// The part that release would emit, for a caller that wants to see it
    /// before deciding.
    pub fn candidate(&self) -> &str {
        &self.inspect[self.inspect.len() - self.releasable..]
    }
}

/// What the buffer is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldbackState {
    /// Accumulating; nothing is waiting on a decision.
    Streaming,
    /// A window is out for inspection and nothing may be released until it
    /// comes back.
    AwaitingDecision,
    /// A window fired. Nothing more is released, ever, from this buffer.
    Blocked,
    /// The consumer went away. Held text is discarded rather than flushed.
    Discarded,
}

/// A sliding, overlapping holdback buffer over one streamed completion.
pub struct Holdback {
    window_bytes: usize,
    overlap_bytes: usize,
    /// Tail of what has already been released, capped at `overlap_bytes`.
    released_tail: String,
    /// Produced but not yet offered for inspection.
    pending: String,
    /// Offered for inspection and awaiting a decision.
    staged: String,
    state: HoldbackState,
    finished: bool,
}

impl Holdback {
    pub fn new(window_bytes: usize, overlap_bytes: usize) -> Self {
        Self {
            window_bytes: window_bytes.max(1),
            overlap_bytes,
            released_tail: String::new(),
            pending: String::new(),
            staged: String::new(),
            state: HoldbackState::Streaming,
            finished: false,
        }
    }

    /// A buffer for a model whose recorded ceiling forbids partial streaming.
    ///
    /// SENT-009: at Confidential or above nothing partial leaves, and the
    /// restriction comes from the ceiling rather than from a request flag, so
    /// there is no argument a caller can pass to get the other behaviour. The
    /// window is effectively unbounded, which means the first window offered is
    /// the final one and the completion is released whole or not at all.
    pub fn for_ceiling(ceiling: SensitivityLevel) -> Self {
        if ceiling >= SensitivityLevel::Confidential {
            return Self {
                window_bytes: usize::MAX,
                overlap_bytes: 0,
                released_tail: String::new(),
                pending: String::new(),
                staged: String::new(),
                state: HoldbackState::Streaming,
                finished: false,
            };
        }
        Self::new(DEFAULT_WINDOW_BYTES, DEFAULT_OVERLAP_BYTES)
    }

    pub fn state(&self) -> HoldbackState {
        self.state
    }

    /// Whether this buffer will ever release anything partial.
    pub fn streams_partial(&self) -> bool {
        self.window_bytes != usize::MAX
    }

    /// Bytes produced but not yet released.
    pub fn held_bytes(&self) -> usize {
        self.pending.len() + self.staged.len()
    }

    /// Accept generated text. Nothing is released here: production and release
    /// are separate steps precisely so that inspection can sit between them.
    pub fn push(&mut self, chunk: &str) {
        if matches!(
            self.state,
            HoldbackState::Blocked | HoldbackState::Discarded
        ) {
            return;
        }
        self.pending.push_str(chunk);
    }

    /// Mark the completion finished, so the last partial window is still
    /// inspected rather than left in the buffer (SENT-007 edge case).
    pub fn finish(&mut self) {
        self.finished = true;
    }

    /// The next window to inspect, if one is ready.
    ///
    /// Returns nothing while a previous window is still out: two windows in
    /// flight would let the second be decided against text the first might
    /// still block.
    pub fn take_window(&mut self) -> Option<Window> {
        if self.state != HoldbackState::Streaming {
            return None;
        }
        if self.pending.is_empty() {
            return None;
        }
        if self.pending.len() < self.window_bytes && !self.finished {
            return None;
        }
        let take = if self.finished {
            self.pending.len()
        } else {
            // Never split a UTF-8 character: a window boundary inside one would
            // produce text no tier can read and no consumer can display. When
            // one character is wider than the whole window, take the character
            // rather than nothing, or the buffer never drains.
            match floor_char_boundary(&self.pending, self.window_bytes) {
                0 => ceil_char_boundary(&self.pending, self.window_bytes),
                take => take,
            }
        };
        self.staged = self.pending.drain(..take).collect();
        self.state = HoldbackState::AwaitingDecision;
        let mut inspect = self.released_tail.clone();
        inspect.push_str(&self.staged);
        Some(Window {
            inspect,
            releasable: self.staged.len(),
            final_window: self.finished && self.pending.is_empty(),
        })
    }

    /// Release the staged window, returning the text the consumer may now see.
    pub fn release(&mut self) -> String {
        if self.state != HoldbackState::AwaitingDecision {
            return String::new();
        }
        let released = std::mem::take(&mut self.staged);
        self.state = HoldbackState::Streaming;
        self.released_tail.push_str(&released);
        if self.released_tail.len() > self.overlap_bytes {
            let cut = self.released_tail.len() - self.overlap_bytes;
            let cut = ceil_char_boundary(&self.released_tail, cut);
            self.released_tail.drain(..cut);
        }
        released
    }

    /// Refuse the staged window. Nothing further is released from this buffer:
    /// a completion whose middle was withheld is not a completion, and emitting
    /// the rest would tell the consumer exactly where the label was.
    pub fn block(&mut self) {
        self.staged.clear();
        self.pending.clear();
        self.state = HoldbackState::Blocked;
    }

    /// The consumer disconnected. Held text is dropped rather than flushed
    /// (SENT-007 edge case): a disconnect is not an inspection.
    pub fn discard(&mut self) {
        self.staged.clear();
        self.pending.clear();
        self.released_tail.clear();
        self.state = HoldbackState::Discarded;
    }
}

impl Default for Holdback {
    fn default() -> Self {
        Self::new(DEFAULT_WINDOW_BYTES, DEFAULT_OVERLAP_BYTES)
    }
}

fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    /// Drive a completion through the buffer, blocking on any window whose
    /// inspection text contains `canary`. Returns what the consumer saw.
    fn stream(holdback: &mut Holdback, chunks: &[&str], canary: Option<&str>) -> String {
        let mut seen = String::new();
        for chunk in chunks {
            holdback.push(chunk);
            while let Some(window) = holdback.take_window() {
                if canary.is_some_and(|c| window.inspect.contains(c)) {
                    holdback.block();
                    return seen;
                }
                seen.push_str(&holdback.release());
            }
        }
        holdback.finish();
        while let Some(window) = holdback.take_window() {
            if canary.is_some_and(|c| window.inspect.contains(c)) {
                holdback.block();
                return seen;
            }
            seen.push_str(&holdback.release());
        }
        seen
    }

    /// SENT-007: tokens are released only after the window covering them has
    /// been inspected.
    #[spec("SENT-007")]
    #[test]
    fn nothing_is_released_before_its_window_is_inspected() {
        let mut holdback = Holdback::new(16, 4);

        holdback.push("short");

        assert_eq!(
            holdback.take_window(),
            None,
            "a partial window is not ready"
        );
        assert_eq!(
            holdback.release(),
            "",
            "and nothing can be released from it"
        );
        assert_eq!(holdback.held_bytes(), 5);
    }

    /// SENT-007: a window that fires is never released.
    #[spec("SENT-007")]
    #[test]
    fn a_window_that_fires_is_never_released() {
        let mut holdback = Holdback::new(16, 4);

        let seen = stream(
            &mut holdback,
            &[
                "harmless prose here ",
                "and then CANARY appears ",
                "and more after it",
            ],
            Some("CANARY"),
        );

        assert!(!seen.contains("CANARY"));
        assert_eq!(holdback.state(), HoldbackState::Blocked);
        // And nothing further comes out of a blocked buffer.
        holdback.push("more text");
        assert_eq!(holdback.take_window(), None);
    }

    /// SENT-007: windows overlap, so a label straddling a boundary is still
    /// seen whole by at least one inspection.
    #[spec("SENT-007")]
    #[test]
    fn a_label_across_a_window_boundary_is_still_inspected_whole() {
        // Sixteen-byte windows with eight bytes of overlap; the canary is
        // placed so that it spans the seam between two windows.
        let mut holdback = Holdback::new(16, 8);
        let text = "aaaaaaaaaaaaaCANARYbbbbbbbbbbbbbbbb";

        let mut inspected = Vec::new();
        holdback.push(text);
        holdback.finish();
        while let Some(window) = holdback.take_window() {
            inspected.push(window.inspect.clone());
            holdback.release();
        }

        assert!(
            inspected.iter().any(|w| w.contains("CANARY")),
            "no window saw the canary whole: {inspected:?}"
        );
    }

    /// SENT-007 edge case: the completion ends mid-window and the final partial
    /// window is still inspected.
    #[spec("SENT-007")]
    #[test]
    fn a_final_partial_window_is_still_inspected() {
        let mut holdback = Holdback::new(64, 8);

        holdback.push("a tail shorter than a window");
        assert_eq!(holdback.take_window(), None);
        holdback.finish();

        let window = holdback.take_window().expect("the tail must be inspected");
        assert!(window.final_window);
        assert_eq!(window.candidate(), "a tail shorter than a window");
    }

    /// SENT-007 edge case: a disconnect discards held text rather than
    /// flushing it. A disconnect is not an inspection.
    #[spec("SENT-007")]
    #[test]
    fn a_disconnect_discards_held_text_rather_than_flushing_it() {
        let mut holdback = Holdback::new(16, 4);
        holdback.push("some held text that has not been inspected");

        holdback.discard();

        assert_eq!(holdback.state(), HoldbackState::Discarded);
        assert_eq!(holdback.held_bytes(), 0);
        holdback.finish();
        assert_eq!(holdback.take_window(), None);
    }

    /// SENT-009: a model whose ceiling is Confidential or above streams nothing
    /// partial, and the restriction comes from the ceiling rather than a flag.
    #[spec("SENT-009")]
    #[test]
    fn a_confidential_ceiling_streams_nothing_partial() {
        let mut holdback = Holdback::for_ceiling(SensitivityLevel::Confidential);

        assert!(!holdback.streams_partial());
        for _ in 0..64 {
            holdback.push("a long stretch of generated text that would fill many windows. ");
            assert_eq!(
                holdback.take_window(),
                None,
                "no window may be offered before the completion is whole"
            );
        }
        holdback.finish();

        let window = holdback.take_window().expect("the whole completion");
        assert!(window.final_window);
    }

    /// SENT-009: a caller cannot opt out. There is no argument that turns
    /// partial streaming back on for a model above the ceiling.
    #[spec("SENT-009")]
    #[test]
    fn the_ceiling_and_not_the_caller_decides_whether_to_stream() {
        assert!(!Holdback::for_ceiling(SensitivityLevel::Restricted).streams_partial());
        assert!(!Holdback::for_ceiling(SensitivityLevel::Confidential).streams_partial());
        assert!(Holdback::for_ceiling(SensitivityLevel::Internal).streams_partial());
        assert!(Holdback::for_ceiling(SensitivityLevel::Public).streams_partial());
    }

    #[test]
    fn a_clean_completion_arrives_whole_and_in_order() {
        let mut holdback = Holdback::new(16, 4);
        let chunks = ["the quick brown ", "fox jumps over ", "the lazy dog"];

        let seen = stream(&mut holdback, &chunks, None);

        assert_eq!(seen, chunks.concat());
    }

    #[test]
    fn a_window_boundary_never_splits_a_character() {
        // A boundary inside a multi-byte character produces text no tier can
        // read and no consumer can display.
        let mut holdback = Holdback::new(5, 2);
        holdback.push("héllo wörld ünicode");
        holdback.finish();

        let mut seen = String::new();
        while let Some(_window) = holdback.take_window() {
            seen.push_str(&holdback.release());
        }

        assert_eq!(seen, "héllo wörld ünicode");
    }

    #[test]
    fn only_one_window_is_in_flight_at_a_time() {
        // Two windows in flight would let the second be decided against text
        // the first might still block.
        let mut holdback = Holdback::new(8, 2);
        holdback.push("enough text for several windows here");

        assert!(holdback.take_window().is_some());
        assert_eq!(holdback.take_window(), None);
    }
}
