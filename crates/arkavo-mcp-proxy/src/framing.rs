//! Line framing for the downstream stdio connection.
//!
//! JSON-RPC over stdio is newline-delimited, and a reader that simply waits
//! for the newline lets one client decide how much the proxy buffers. This
//! reads a line at a time against [`MAX_LINE_BYTES`], discarding — rather
//! than accumulating — anything past it, so an over-long message costs
//! bounded memory and the connection survives to answer the next one.

use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// The longest single JSON-RPC line the proxy accepts from the downstream
/// client. MCP messages are small; a megabyte is far above any real call and
/// far below what would let a client exhaust memory.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// What one read of the downstream connection produced.
#[derive(Debug, PartialEq, Eq)]
pub enum Line {
    /// A complete message, at most [`MAX_LINE_BYTES`] bytes.
    Message(String),
    /// The line was longer than [`MAX_LINE_BYTES`] and has been skipped to
    /// its end; the connection is still usable.
    TooLong,
    /// The client closed the connection.
    Eof,
}

/// Read one newline-delimited message, bounded by [`MAX_LINE_BYTES`].
///
/// A final line without a trailing newline is returned as a message, the
/// same as `read_line` would.
pub async fn read_line<R: AsyncBufRead + Unpin>(reader: &mut R) -> std::io::Result<Line> {
    let mut buffered: Vec<u8> = Vec::new();
    let mut discarding = false;

    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(if discarding {
                Line::TooLong
            } else if buffered.is_empty() {
                Line::Eof
            } else {
                message(buffered)?
            });
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.unwrap_or(available.len());
        if !discarding {
            if buffered.len() + take > MAX_LINE_BYTES {
                // Past the cap: keep nothing, and read on only far enough to
                // find where this message ends.
                discarding = true;
                buffered = Vec::new();
            } else {
                buffered.extend_from_slice(&available[..take]);
            }
        }
        let consumed = newline.map_or(available.len(), |index| index + 1);
        reader.consume(consumed);

        if newline.is_some() {
            return Ok(if discarding {
                Line::TooLong
            } else {
                message(buffered)?
            });
        }
    }
}

fn message(bytes: Vec<u8>) -> std::io::Result<Line> {
    String::from_utf8(bytes).map(Line::Message).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "stream did not contain valid UTF-8",
        )
    })
}

#[cfg(test)]
// The `#[tokio::test]` macro expands to `Runtime::block_on`, which
// `.clippy.toml` disallows outside test code.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    async fn read_all(input: Vec<u8>) -> Vec<Line> {
        let mut reader = BufReader::new(std::io::Cursor::new(input));
        let mut lines = Vec::new();
        loop {
            let line = read_line(&mut reader).await.expect("read");
            let done = line == Line::Eof;
            lines.push(line);
            if done {
                return lines;
            }
        }
    }

    #[tokio::test]
    async fn reads_messages_and_reports_eof() {
        assert_eq!(
            read_all(b"{\"a\":1}\n{\"b\":2}\n".to_vec()).await,
            vec![
                Line::Message("{\"a\":1}".to_string()),
                Line::Message("{\"b\":2}".to_string()),
                Line::Eof,
            ]
        );
        // A last line with no trailing newline is still a message.
        assert_eq!(
            read_all(b"{\"a\":1}".to_vec()).await,
            vec![Line::Message("{\"a\":1}".to_string()), Line::Eof]
        );
        assert_eq!(read_all(Vec::new()).await, vec![Line::Eof]);
    }

    /// The cap is on one message, not on the connection: an over-long line is
    /// reported and skipped, and the message after it is read normally.
    #[tokio::test]
    async fn an_over_long_line_is_skipped_and_the_next_one_survives() {
        let mut input = vec![b'x'; MAX_LINE_BYTES + 1];
        input.push(b'\n');
        input.extend_from_slice(b"{\"next\":true}\n");

        assert_eq!(
            read_all(input).await,
            vec![
                Line::TooLong,
                Line::Message("{\"next\":true}".to_string()),
                Line::Eof,
            ]
        );
    }

    /// Exactly at the cap is still a message: the bound is what may be
    /// buffered, and the newline itself is not part of it.
    #[tokio::test]
    async fn a_line_at_the_cap_is_a_message() {
        let mut input = vec![b'x'; MAX_LINE_BYTES];
        input.push(b'\n');
        match read_all(input).await.first() {
            Some(Line::Message(text)) => assert_eq!(text.len(), MAX_LINE_BYTES),
            other => panic!("expected a message at the cap, got {other:?}"),
        }
    }

    /// An over-long line that never ends must not be buffered while the
    /// client keeps writing: it is consumed to EOF and reported once.
    #[tokio::test]
    async fn an_unterminated_over_long_line_ends_at_eof() {
        let input = vec![b'x'; MAX_LINE_BYTES * 2];
        assert_eq!(read_all(input).await, vec![Line::TooLong, Line::Eof]);
    }

    #[tokio::test]
    async fn invalid_utf8_is_an_io_error() {
        let mut reader = BufReader::new(std::io::Cursor::new(b"\xff\xfe\n".to_vec()));
        let error = read_line(&mut reader).await.expect_err("invalid UTF-8");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }
}
