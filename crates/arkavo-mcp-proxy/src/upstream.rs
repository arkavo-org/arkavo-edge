//! Connection to the single upstream MCP server, spawned as a subprocess
//! and spoken to over stdio with raw JSON-RPC messages.
//!
//! Unlike `arkavo_mcp_runtime::McpClient`, this connection performs no
//! handshake of its own and returns the upstream response object verbatim,
//! so the proxy can relay the downstream client's `initialize` and preserve
//! upstream error codes and messages exactly.
//!
//! Traffic flows one way: the downstream client asks, the upstream server
//! answers. A server-initiated request — `sampling/createMessage`,
//! `elicitation/create`, `roots/list` — is **not** relayed to the client in
//! this slice, because the client's permit and proof cover the call it made
//! and nothing the server thinks of afterwards. Such a request is answered
//! here with JSON-RPC `-32601`, so the upstream learns immediately instead of
//! blocking until its own timeout. Server notifications, which expect no
//! answer, are logged and dropped.
//!
//! What an upstream message *is* is decided by its shape and never by its
//! id: anything carrying `method` — whatever type that field has — is the
//! server asking, and is refused or dropped whatever id it names. Only a
//! message with no `method` at all is an answer, and only then is it matched
//! against the requests in flight.
//!
//! The upstream is untrusted in the same way the downstream client is, so
//! what it can make the proxy hold is bounded the same way: its output is
//! read one [`MAX_LINE_BYTES`] line at a time, and the refusals it can
//! provoke queue up to `refusals::REFUSAL_QUEUE_DEPTH` and are written by
//! a task of their own, so a server that floods requests and stops reading
//! its own stdin cannot stall the reading of the response a caller is
//! waiting for. That queue, and the routing that feeds it, live in
//! [`crate::refusals`].
//!
//! Writing is bounded in time as well. The writer task and every request
//! share one stdin behind one mutex, so a server that stops reading blocks
//! whoever holds that lock and everyone waiting for it — including a
//! `request` that has not started its receive timeout yet. Each write is
//! therefore wrapped in the connection's timeout: on expiry it is abandoned,
//! the connection is marked closed (a partial line is already in the pipe,
//! and the next write would splice onto it), and a request reports
//! [`UpstreamError::WriteTimeout`], which counts as "may have run".
//!
//! Closing is final, and it is final for everyone: the requests still
//! waiting for a response are failed at that moment, exactly as the reader
//! task fails them at EOF, rather than each sitting out a receive timeout on
//! a connection nothing will be written to again.

// `pub(crate)` is the real, intended visibility here (the module is private,
// so nothing leaks past the crate either way); `redundant_pub_crate` wants
// `pub`, which `unreachable_pub` then rejects.
#![allow(clippy::redundant_pub_crate)]

use crate::framing::{self, Line, MAX_LINE_BYTES};
use crate::refusals;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, mpsc, oneshot};
use tracing::{debug, warn};

/// Errors from the upstream connection.
#[derive(Debug, thiserror::Error)]
pub enum UpstreamError {
    /// The upstream server process could not be spawned.
    #[error("failed to spawn upstream server '{command}': {source}")]
    Spawn {
        /// Command that failed to spawn.
        command: String,
        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// The upstream connection was already closed when the request was
    /// made, so nothing was sent.
    #[error("upstream connection closed")]
    Closed,

    /// The upstream closed its stdout after the request was written, so it
    /// may have read and run the call before it went away.
    #[error("upstream connection closed after the request was sent")]
    ClosedAfterSend,

    /// Writing the request to the upstream server's stdin failed part-way,
    /// so the line it would dispatch on never arrived complete.
    #[error("upstream write failed: {0}")]
    Write(String),

    /// Flushing the request failed after every byte of it, the terminating
    /// newline included, had been written, so the upstream may already have
    /// read and run the call.
    #[error("upstream flush failed: {0}")]
    Flush(String),

    /// Writing the request to the upstream server's stdin did not finish in
    /// time, because the server stopped reading it.
    ///
    /// Whatever the pipe accepted before it filled *was* delivered, so the
    /// upstream may have read a whole line and run it; the write is abandoned
    /// pessimistically rather than held open.
    #[error("upstream write timed out after {0:?}")]
    WriteTimeout(Duration),

    /// The upstream server did not respond in time. The request was
    /// dispatched; a slow tool goes on running after the wait is abandoned.
    #[error("upstream request timed out after {0:?}")]
    Timeout(Duration),
}

impl UpstreamError {
    /// Whether the request may have reached the upstream server and run
    /// there.
    ///
    /// This is the question a caller that spent something to admit the call
    /// has to answer before handing it back. It is deliberately
    /// pessimistic: only the failures that happen strictly before the
    /// request is on the wire — the connection already gone, the write
    /// itself cut short — are reported as "never ran".
    pub fn may_have_reached_upstream(&self) -> bool {
        match self {
            Self::Spawn { .. } | Self::Closed | Self::Write(_) => false,
            Self::ClosedAfterSend | Self::Flush(_) | Self::WriteTimeout(_) | Self::Timeout(_) => {
                true
            }
        }
    }
}

/// Default per-request timeout when the caller does not configure one.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Key used to correlate responses with pending requests. JSON-RPC allows
/// string or numeric ids; the serialized form is a stable key for both.
pub(crate) fn id_key(id: &Value) -> String {
    serde_json::to_string(id).unwrap_or_else(|_| "null".to_string())
}

/// A spawned upstream MCP server reached over stdio.
pub struct UpstreamConnection {
    child: Mutex<Child>,
    /// Shared with the task that writes refusals of server-initiated
    /// requests.
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    connected: Arc<AtomicBool>,
    timeout: Duration,
}

impl UpstreamConnection {
    /// Spawn `command args` and connect to its stdio.
    pub fn spawn(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        timeout: Option<Duration>,
    ) -> Result<Self, UpstreamError> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        for (key, value) in env {
            cmd.env(key, value);
        }

        let timeout = timeout.unwrap_or(DEFAULT_TIMEOUT);
        let mut child = cmd.spawn().map_err(|source| UpstreamError::Spawn {
            command: command.to_string(),
            source,
        })?;

        let stdin = Arc::new(Mutex::new(child.stdin.take().ok_or(UpstreamError::Closed)?));
        let stdout = child.stdout.take().ok_or(UpstreamError::Closed)?;

        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let connected = Arc::new(AtomicBool::new(true));

        // Writer task: the only place refusals are written, so the reader
        // never waits on an upstream that has stopped reading its own stdin.
        // It ends when the reader task drops the sender at EOF, or when a
        // write of its own runs out of time.
        let (refusals, queued) = mpsc::channel::<Value>(refusals::REFUSAL_QUEUE_DEPTH);
        refusals::spawn_writer(Arc::clone(&stdin), queued, timeout, Arc::clone(&connected));

        // Reader task: correlate responses by id, fail all pending requests
        // when the upstream closes its stdout.
        let reader_pending = Arc::clone(&pending);
        let reader_connected = Arc::clone(&connected);
        tokio::spawn(async move {
            let mut stdout = BufReader::new(stdout);
            // What this reader has refused and what it had to drop.
            // Counted on every occurrence, warned about on the first and then
            // at each doubling, so a flood of any size costs a dozen log
            // lines rather than one per refusal.
            let mut refusal_counts = refusals::RefusalCounts::default();
            loop {
                match framing::read_line(&mut stdout).await {
                    Ok(Line::Message(line)) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<Value>(trimmed) {
                            Ok(message) => {
                                refusals::dispatch(
                                    &reader_pending,
                                    &refusals,
                                    message,
                                    &mut refusal_counts,
                                )
                                .await;
                            }
                            Err(e) => warn!("unparseable upstream output: {e}"),
                        }
                    }
                    // One line the proxy will not buffer is not a reason to
                    // drop the connection: it is discarded and reading goes on.
                    Ok(Line::TooLong) => warn!(
                        max_bytes = MAX_LINE_BYTES,
                        "discarded an over-long upstream line"
                    ),
                    Ok(Line::Eof) => break,
                    Err(e) => {
                        warn!("upstream read failed: {e}");
                        break;
                    }
                }
            }
            reader_connected.store(false, Ordering::SeqCst);
            // Dropping the senders resolves every pending receiver with an
            // error instead of leaving callers to wait out the timeout.
            reader_pending.lock().await.clear();
        });

        Ok(Self {
            child: Mutex::new(child),
            stdin,
            pending,
            connected,
            timeout,
        })
    }

    /// Write one message to the upstream's stdin, bounded by this
    /// connection's timeout.
    ///
    /// The bound covers waiting for the shared stdin lock as well as the
    /// write itself, because `write_line` takes that lock: an upstream that
    /// has stopped reading blocks whoever holds it, and everyone behind them
    /// would otherwise wait with no timeout of their own.
    ///
    /// A write abandoned part-way may have left a prefix of the line in the
    /// pipe, and the next write would splice onto it, so the connection is
    /// marked closed rather than spoken on again.
    ///
    /// Retiring the connection also fails everything still waiting on it,
    /// the way the reader task does at EOF. Nothing will be written on this
    /// pipe again, so a request already past its own write is waiting for an
    /// answer on a connection that is over; dropping its sender tells it now
    /// rather than leaving it to sit out its receive timeout. Each such
    /// waiter sees [`UpstreamError::ClosedAfterSend`] — its own bytes did
    /// reach the pipe, so its call may have run — and keeps whatever was
    /// spent admitting it.
    async fn write_bounded(&self, message: &Value) -> Result<(), UpstreamError> {
        match tokio::time::timeout(self.timeout, write_line(&self.stdin, message)).await {
            Ok(result) => result,
            Err(_) => {
                warn!(
                    timeout = ?self.timeout,
                    "upstream stopped reading its stdin; the write was abandoned"
                );
                self.connected.store(false, Ordering::SeqCst);
                self.pending.lock().await.clear();
                Err(UpstreamError::WriteTimeout(self.timeout))
            }
        }
    }

    /// Send a request and return the full JSON-RPC response object,
    /// including any upstream error, verbatim.
    pub async fn request(
        &self,
        id: &Value,
        method: &str,
        params: Option<&Value>,
    ) -> Result<Value, UpstreamError> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(UpstreamError::Closed);
        }

        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params.cloned().unwrap_or_else(|| json!({})),
        });
        let key = id_key(id);

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(key.clone(), tx);
        // Re-check after inserting: the reader task may have observed
        // upstream EOF between the first check and the insert and already
        // cleared pending — without this the request would hang until the
        // timeout instead of failing fast.
        if !self.connected.load(Ordering::SeqCst) {
            self.pending.lock().await.remove(&key);
            return Err(UpstreamError::Closed);
        }
        if let Err(e) = self.write_bounded(&request).await {
            self.pending.lock().await.remove(&key);
            return Err(e);
        }

        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(UpstreamError::ClosedAfterSend),
            Err(_) => {
                self.pending.lock().await.remove(&key);
                Err(UpstreamError::Timeout(self.timeout))
            }
        }
    }

    /// Send a notification; no response is expected.
    pub async fn notify(&self, method: &str, params: Option<&Value>) -> Result<(), UpstreamError> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(UpstreamError::Closed);
        }
        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params.cloned().unwrap_or_else(|| json!({})),
        });
        self.write_bounded(&notification).await
    }

    /// Terminate the upstream server process.
    pub async fn shutdown(&self) {
        self.connected.store(false, Ordering::SeqCst);
        if let Err(e) = self.child.lock().await.kill().await {
            debug!("upstream process already exited: {e}");
        }
    }
}

/// Write one JSON-RPC message and its newline to `stdin`.
///
/// Generic over the writer so the refusal writer can be exercised against
/// a stream a test controls; in this crate it is only ever a
/// [`tokio::process::ChildStdin`]. Unbounded on its own: every caller
/// wraps it in a timeout, because the lock it takes is shared.
pub(crate) async fn write_line<W: tokio::io::AsyncWrite + Unpin>(
    stdin: &Mutex<W>,
    message: &Value,
) -> Result<(), UpstreamError> {
    let mut bytes = serde_json::to_vec(message).unwrap_or_default();
    bytes.push(b'\n');
    let mut stdin = stdin.lock().await;
    stdin
        .write_all(&bytes)
        .await
        .map_err(|e| UpstreamError::Write(e.to_string()))?;
    // The write above put the whole line, newline included, into the pipe:
    // a failure from here on cannot promise the upstream never saw it.
    stdin
        .flush()
        .await
        .map_err(|e| UpstreamError::Flush(e.to_string()))
}

impl std::fmt::Debug for UpstreamConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpstreamConnection")
            .field("connected", &self.connected.load(Ordering::SeqCst))
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
// The `#[tokio::test]` macro expands to `Runtime::block_on`, which
// `.clippy.toml` disallows outside test code.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Regression: after the upstream exits, a request must fail fast with
    /// `Closed` — never hang for the full timeout because the reader task
    /// cleared `pending` before the request was inserted.
    #[tokio::test]
    async fn request_fails_closed_fast_after_upstream_exit() {
        // `true` exits immediately without reading or writing anything.
        let conn = UpstreamConnection::spawn("true", &[], &HashMap::new(), None).unwrap();

        // Wait for the reader task to observe EOF and mark disconnected.
        for _ in 0..200 {
            if !conn.connected.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            !conn.connected.load(Ordering::SeqCst),
            "reader task must mark the connection closed after upstream EOF"
        );

        let start = Instant::now();
        let err = conn
            .request(&json!(1), "tools/call", None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, UpstreamError::Closed),
            "request after upstream exit must fail with Closed, got {err}"
        );
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "request after upstream exit must fail fast, not wait out the timeout"
        );
        assert!(
            !err.may_have_reached_upstream(),
            "nothing was written, so the call provably never ran upstream"
        );
    }

    /// The other half of the distinction a caller needs: a request that *was*
    /// written and then got no answer because the upstream went away is not
    /// the same failure. The bytes were on the wire, so the call may have run
    /// there, and whatever was spent admitting it must stay spent.
    #[tokio::test]
    async fn a_request_the_upstream_took_before_dying_may_have_run() {
        // Reads exactly one line, answers nothing, exits: the request is
        // written and read, and only then does the connection go away.
        let conn = UpstreamConnection::spawn(
            "sh",
            &["-c".to_string(), "read line".to_string()],
            &HashMap::new(),
            Some(Duration::from_secs(10)),
        )
        .unwrap();

        let err = conn
            .request(&json!(1), "tools/call", None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, UpstreamError::ClosedAfterSend),
            "a request that was sent must not report as never sent, got {err}"
        );
        assert!(err.may_have_reached_upstream());
    }

    /// Retirement is the end of the connection for everyone on it, not only
    /// for the write that could not finish. A request already past its own
    /// write is waiting for an answer on a pipe that will never be written
    /// to again, so it is failed when the connection is retired — the same
    /// end the reader task gives every waiter at EOF — instead of sitting
    /// out a receive timeout of its own on top of the write's.
    ///
    /// The waiter is planted rather than issued as a second request, because
    /// one shared timeout leaves nothing to observe: writes serialize on the
    /// stdin lock, so a stalling write can only begin after an earlier
    /// request's write finished, and its deadline therefore never falls
    /// before that request's own. A sender in `pending` is precisely what a
    /// request past its write leaves behind, so that is what is left there.
    #[tokio::test]
    async fn a_write_timeout_fails_the_requests_already_waiting_on_the_connection() {
        // `sleep` never reads its stdin, so the pipe fills and stays full.
        let conn = UpstreamConnection::spawn(
            "sh",
            &["-c".to_string(), "sleep 30".to_string()],
            &HashMap::new(),
            Some(Duration::from_millis(200)),
        )
        .unwrap();
        let waiting = {
            let (tx, rx) = oneshot::channel();
            conn.pending.lock().await.insert(id_key(&json!(1)), tx);
            rx
        };

        // Past any pipe buffer, so this write cannot finish and the
        // connection is retired on its timeout.
        let padded = json!({"pad": "x".repeat(2 * 1024 * 1024)});
        let err = conn
            .request(&json!(2), "tools/call", Some(&padded))
            .await
            .unwrap_err();
        assert!(
            matches!(err, UpstreamError::WriteTimeout(_)),
            "the stalled write must report as a write timeout, got {err}"
        );

        // Already resolved: the waiter was failed with the connection, not
        // left to discover it a receive timeout later.
        let waited = tokio::time::timeout(Duration::from_millis(100), waiting)
            .await
            .expect("the retirement itself must fail the waiter, not a timeout of its own");
        assert!(waited.is_err(), "a retired connection answers nobody");
        assert!(
            !conn.connected.load(Ordering::SeqCst),
            "the connection is closed once a write of its own is abandoned"
        );
    }
}
