//! One server-sent-events reader for every streaming provider in this crate.
//!
//! Decoding happens on bytes and completes whole events, so a UTF-8 sequence,
//! a CRLF pair or a multi-line `data:` field split across TCP chunks retains
//! its exact meaning. Providers layer their own event vocabulary on top; the
//! framing, the stall timeout and the ownership of the HTTP body live here.

use crate::{Error, Result};
use futures::{Stream, StreamExt};
use std::collections::VecDeque;
use std::pin::Pin;
use std::time::Duration;

/// Refuse to buffer more than this for one event, so a broken or hostile
/// endpoint cannot make the client hold an unbounded reply in memory.
const MAX_EVENT_BYTES: usize = 8 * 1024 * 1024;

/// Decode complete events rather than individual network chunks, so fragmented
/// UTF-8, CRLF and multi-line data fields retain their exact meaning.
#[derive(Default)]
pub(crate) struct Decoder {
    pending: Vec<u8>,
    data: String,
    events: VecDeque<String>,
}

impl Decoder {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<()> {
        self.pending.extend_from_slice(bytes);
        let mut consumed = 0;
        while let Some(offset) = self.pending[consumed..].iter().position(|b| *b == b'\n') {
            let end = consumed + offset;
            let line = std::str::from_utf8(&self.pending[consumed..end])
                .map_err(|_| Error::Stream("Invalid UTF-8 in stream event".into()))?
                .trim_end_matches('\r');
            if line.is_empty() {
                if !self.data.is_empty() {
                    self.events.push_back(std::mem::take(&mut self.data));
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                let data = data.strip_prefix(' ').unwrap_or(data);
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(data);
                if self.data.len() > MAX_EVENT_BYTES {
                    return Err(Error::Stream("Stream event exceeds size limit".into()));
                }
            }
            consumed = end + 1;
        }
        self.pending.drain(..consumed);
        if self.pending.len() > MAX_EVENT_BYTES {
            return Err(Error::Stream("Stream event exceeds size limit".into()));
        }
        Ok(())
    }

    /// The next complete event's `data` payload, if one has been framed.
    pub(crate) fn next_event(&mut self) -> Option<String> {
        self.events.pop_front()
    }
}

/// A response body read one complete event at a time.
///
/// The reader owns the body and is only advanced when a caller asks for the
/// next event, so dropping the consumer drops the connection: no detached task
/// keeps a paid-for generation running.
pub(crate) struct EventStream {
    source: Pin<Box<dyn Stream<Item = std::result::Result<bytes::Bytes, reqwest::Error>> + Send>>,
    decoder: Decoder,
    idle: Duration,
}

impl EventStream {
    pub(crate) fn new<S>(body: S, idle: Duration) -> Self
    where
        S: Stream<Item = std::result::Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
    {
        Self {
            source: Box::pin(body),
            decoder: Decoder::default(),
            idle,
        }
    }

    /// The next complete event, or `Ok(None)` when the body ends.
    ///
    /// A body that stops delivering for longer than the idle bound fails on its
    /// own evidence rather than holding the caller for the whole request budget.
    /// A trailing event with no terminating blank line is not an event.
    pub(crate) async fn next_event(&mut self) -> Result<Option<String>> {
        loop {
            if let Some(data) = self.decoder.next_event() {
                return Ok(Some(data));
            }
            let next = tokio::time::timeout(self.idle, self.source.next())
                .await
                .map_err(|_| Error::Stream("SSE stream stalled".into()))?;
            match next {
                Some(Ok(bytes)) => self.decoder.push(&bytes)?,
                Some(Err(error)) => return Err(Error::Request(error.without_url())),
                None => return Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[arkavo_test_macros::spec("ASTRA-003")]
    #[test]
    fn fragmented_utf8_crlf_and_multiline_data_are_lossless() {
        let input = "data: {\r\ndata: \"type\":\"response.output_text.delta\",\r\ndata: \"delta\":\"🌍\"}\r\n\r\n";
        let mut decoder = Decoder::default();
        for byte in input.as_bytes() {
            decoder.push(&[*byte]).unwrap();
        }
        // Continuation lines join with a newline, as the wire format defines;
        // the character split across chunks arrives whole.
        assert_eq!(
            decoder.next_event().as_deref(),
            Some("{\n\"type\":\"response.output_text.delta\",\n\"delta\":\"🌍\"}")
        );
        assert!(decoder.next_event().is_none());
    }

    /// Replacing an undecodable byte would hand the provider's parser text the
    /// endpoint never sent.
    #[arkavo_test_macros::spec("ASTRA-003")]
    #[test]
    fn invalid_utf8_is_not_replaced() {
        assert!(Decoder::default().push(b"data: \xff\n\n").is_err());
    }

    /// A `data:` line without the optional space, and a comment or field this
    /// crate does not read, are both part of the wire format.
    #[arkavo_test_macros::spec("ASTRA-003")]
    #[test]
    fn optional_space_and_unread_fields_follow_the_wire_format() {
        let mut decoder = Decoder::default();
        decoder
            .push(b": keep-alive\nevent: message\ndata:tight\nid: 7\n\n")
            .unwrap();
        assert_eq!(decoder.next_event().as_deref(), Some("tight"));
    }

    #[arkavo_test_macros::spec("ASTRA-003")]
    #[test]
    fn an_unterminated_event_is_not_delivered() {
        let mut decoder = Decoder::default();
        decoder.push(b"data: half").unwrap();
        assert!(decoder.next_event().is_none());
        decoder.push(b"\n\n").unwrap();
        assert_eq!(decoder.next_event().as_deref(), Some("half"));
    }

    #[arkavo_test_macros::spec("ASTRA-003")]
    #[test]
    fn an_oversized_event_fails_before_it_is_buffered() {
        let mut decoder = Decoder::default();
        let line = format!("data: {}\n", "a".repeat(1024 * 1024));
        let mut failed = false;
        for _ in 0..16 {
            if decoder.push(line.as_bytes()).is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed, "an unbounded event must not be buffered");
    }

    #[tokio::test]
    async fn a_body_that_ends_reports_the_end_rather_than_an_event() {
        let mut events = EventStream::new(
            futures::stream::iter(vec![Ok(bytes::Bytes::from_static(
                b"data: one\n\ndata: cut",
            ))]),
            Duration::from_secs(1),
        );
        assert_eq!(events.next_event().await.unwrap().as_deref(), Some("one"));
        assert!(events.next_event().await.unwrap().is_none());
    }

    /// A connection that stops delivering must fail on its own evidence.
    #[tokio::test]
    async fn a_stalled_body_fails_on_the_idle_bound() {
        let error = EventStream::new(futures::stream::pending(), Duration::from_millis(20))
            .next_event()
            .await
            .unwrap_err();
        assert!(error.to_string().contains("stalled"), "{error}");
    }
}
