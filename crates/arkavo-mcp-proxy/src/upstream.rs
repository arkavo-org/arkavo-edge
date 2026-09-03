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
//! provoke queue up to [`REFUSAL_QUEUE_DEPTH`] and are written by a task of
//! their own, so a server that floods requests and stops reading its own
//! stdin cannot stall the reading of the response a caller is waiting for.
//!
//! Writing is bounded in time as well. The writer task and every request
//! share one stdin behind one mutex, so a server that stops reading blocks
//! whoever holds that lock and everyone waiting for it — including a
//! `request` that has not started its receive timeout yet. Each write is
//! therefore wrapped in the connection's timeout: on expiry it is abandoned,
//! the connection is marked closed (a partial line is already in the pipe,
//! and the next write would splice onto it), and a request reports
//! [`UpstreamError::WriteTimeout`], which counts as "may have run".

use crate::framing::{self, Line, MAX_LINE_BYTES};
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

/// JSON-RPC method-not-found, the answer to a server-initiated request this
/// slice does not relay to the downstream client.
const METHOD_NOT_FOUND: i64 = -32601;

/// How many refusals of server-initiated requests wait to be written before
/// further ones are dropped.
///
/// A well-behaved server has at most one question outstanding at a time. A
/// flood is a broken or hostile server, and the queue is what keeps answering
/// it from mattering: the reader hands refusals over without waiting, and if
/// the writer cannot keep up — an upstream that asks without ever reading its
/// own stdin — the refusals are dropped and counted rather than allowed to
/// block the response the proxy is actually waiting for.
const REFUSAL_QUEUE_DEPTH: usize = 16;

/// Key used to correlate responses with pending requests. JSON-RPC allows
/// string or numeric ids; the serialized form is a stable key for both.
fn id_key(id: &Value) -> String {
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
        let (refusals, queued) = mpsc::channel::<Value>(REFUSAL_QUEUE_DEPTH);
        spawn_refusal_writer(Arc::clone(&stdin), queued, timeout, Arc::clone(&connected));

        // Reader task: correlate responses by id, fail all pending requests
        // when the upstream closes its stdout.
        let reader_pending = Arc::clone(&pending);
        let reader_connected = Arc::clone(&connected);
        tokio::spawn(async move {
            let mut stdout = BufReader::new(stdout);
            // Refusals the queue had no room for. Counted on every drop
            // but warned about on the first and then at each doubling, so a
            // flood of any size costs a dozen log lines rather than one per
            // refusal.
            let mut dropped_refusals = 0u64;
            loop {
                match framing::read_line(&mut stdout).await {
                    Ok(Line::Message(line)) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<Value>(trimmed) {
                            Ok(message) => {
                                dispatch(
                                    &reader_pending,
                                    &refusals,
                                    message,
                                    &mut dropped_refusals,
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
    async fn write_bounded(&self, message: &Value) -> Result<(), UpstreamError> {
        match tokio::time::timeout(self.timeout, write_line(&self.stdin, message)).await {
            Ok(result) => result,
            Err(_) => {
                warn!(
                    timeout = ?self.timeout,
                    "upstream stopped reading its stdin; the write was abandoned"
                );
                self.connected.store(false, Ordering::SeqCst);
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

/// Route one message from the upstream server by its shape.
///
/// The order matters, and it is the whole point of this function. A message
/// carrying `method` is something the *server* is asking for — a request when
/// it also carries an id, a notification when it does not — and is never an
/// answer to anything this side sent, whatever id it names. Matching on the
/// id first is how a hostile upstream reuses an in-flight id to have a
/// `sampling/createMessage` handed to the caller waiting on it and relayed to
/// the downstream client as though it were the tool's own result. Requests
/// are refused here; only a message with no `method` is looked up in
/// `pending`.
async fn dispatch(
    pending: &Mutex<HashMap<String, oneshot::Sender<Value>>>,
    refusals: &mpsc::Sender<Value>,
    message: Value,
    dropped_refusals: &mut u64,
) {
    let id = message.get("id").filter(|value| !value.is_null()).cloned();

    // Presence decides, not type. `"method": 123` or `"method": ["x"]` is
    // still the server naming a method — badly — and reading the name with
    // `as_str` first would let either fall through to the response branch and
    // be relayed to the client as an answer. The name is only for the
    // refusal's text, so a non-string one is described rather than parsed.
    if message.get("method").is_some() {
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("<non-string method>");
        match id {
            Some(id) => refuse_server_request(refusals, &id, method, dropped_refusals),
            None => debug!(method, "upstream notification (dropped)"),
        }
        return;
    }

    let Some(id) = id else {
        warn!("upstream message with neither a method nor an id (dropped)");
        return;
    };
    let sender = pending.lock().await.remove(&id_key(&id));
    match sender {
        Some(sender) => {
            let _ = sender.send(message);
        }
        None => warn!("upstream response with an id nothing is waiting on (dropped)"),
    }
}

/// Answer a request the upstream server made of us.
///
/// The proxy does not relay server-initiated requests to the downstream
/// client, so it refuses them here rather than letting the server wait out
/// its own timeout. The id is echoed back as the server sent it — including
/// when the server reused an id this side has a request in flight on, which
/// is a collision the server made and has to sort out.
///
/// The refusal is queued, never written here: the reader task's job is to
/// keep reading, and a server that asks faster than it reads its own stdin
/// must not be able to stop it. When the queue is full — or the writer task
/// has gone with the connection — the refusal is dropped and counted, which
/// costs that server nothing but its own timeout. Every drop is counted;
/// only the first and each doubling after it is logged, so a flood cannot
/// turn the log into the flood.
fn refuse_server_request(
    refusals: &mpsc::Sender<Value>,
    id: &Value,
    method: &str,
    dropped: &mut u64,
) {
    let refusal = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": METHOD_NOT_FOUND,
            "message": format!(
                "this proxy does not relay server-initiated requests to the client ({method})"
            ),
        },
    });
    if refusals.try_send(refusal).is_ok() {
        warn!(
            method,
            "refusing a server-initiated request: not relayed to the downstream client"
        );
    } else {
        *dropped += 1;
        // A flood is thousands of refusals, and one warning each would make
        // the log the denial of service the queue is there to prevent. The
        // first drop is reported, and then every doubling: enough to see one
        // is happening and roughly how big it got, at a dozen lines for a
        // flood of any size.
        if dropped.is_power_of_two() {
            warn!(
                method,
                dropped = *dropped,
                queue_depth = REFUSAL_QUEUE_DEPTH,
                "dropped the refusal of a server-initiated request: the refusal queue is full"
            );
        }
    }
}

/// The task that writes refusals, and the only place they are written.
///
/// Each write is bounded by `timeout`, because the shared stdin lock is
/// taken inside it: against an upstream that has stopped reading, an
/// unbounded `write_all` here would hold that lock forever and every request
/// behind it would block on the lock rather than on its own timeout. When a
/// write does run out of time the connection is marked closed and the task
/// stops — nothing more can be said on a pipe nobody is reading, and a
/// partial line is already in it.
fn spawn_refusal_writer<W>(
    stdin: Arc<Mutex<W>>,
    mut queued: mpsc::Receiver<Value>,
    timeout: Duration,
    connected: Arc<AtomicBool>,
) where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        while let Some(refusal) = queued.recv().await {
            match tokio::time::timeout(timeout, write_line(&stdin, &refusal)).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => warn!("failed to refuse a server-initiated request: {e}"),
                Err(_) => {
                    warn!(
                        ?timeout,
                        "upstream stopped reading its stdin; no further refusals are written"
                    );
                    connected.store(false, Ordering::SeqCst);
                    return;
                }
            }
        }
    });
}

async fn write_line<W: tokio::io::AsyncWrite + Unpin>(
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
    use arkavo_test_macros::spec;
    use std::time::Instant;

    /// The shape rule at the level it is decided: a message carrying `method`
    /// is the server asking, so it never resolves a request in flight, even
    /// when it names one. Delivering it would hand the caller — and through
    /// it the downstream client — a request the server made, dressed as the
    /// answer to the call the client actually authorized.
    #[tokio::test]
    #[spec("PDG-011")]
    async fn a_server_request_never_resolves_a_pending_id() {
        let pending: Mutex<HashMap<String, oneshot::Sender<Value>>> = Mutex::new(HashMap::new());
        let (sender, receiver) = oneshot::channel();
        let key = id_key(&json!(1));
        pending.lock().await.insert(key.clone(), sender);
        let (refusals, mut queued) = mpsc::channel(REFUSAL_QUEUE_DEPTH);
        let mut dropped = 0u64;

        dispatch(
            &pending,
            &refusals,
            json!({"jsonrpc": "2.0", "id": 1, "method": "sampling/createMessage"}),
            &mut dropped,
        )
        .await;

        assert!(
            pending.lock().await.contains_key(&key),
            "the request must still be in flight, waiting for a real answer"
        );
        let refusal = queued.try_recv().expect("the request is refused");
        assert_eq!(refusal["id"], 1);
        assert_eq!(refusal["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(dropped, 0);

        // A `method` that is not a string is still the server asking. Reading
        // the name before deciding made both of these fall through to the
        // response branch, where they resolved the pending id and were
        // relayed to the client as the tool's own answer.
        for method in [json!(1), json!(["sampling/createMessage"])] {
            dispatch(
                &pending,
                &refusals,
                json!({"jsonrpc": "2.0", "id": 1, "method": method}),
                &mut dropped,
            )
            .await;
            assert!(
                pending.lock().await.contains_key(&key),
                "a non-string method must not resolve a request in flight: {method}"
            );
            let refusal = queued.try_recv().expect("it is refused like any other");
            assert_eq!(refusal["id"], 1);
            assert_eq!(refusal["error"]["code"], METHOD_NOT_FOUND);
            let message = refusal["error"]["message"].as_str().expect("a message");
            assert!(
                message.contains("<non-string method>"),
                "the refusal names what it could not read: {message}"
            );
        }
        assert_eq!(dropped, 0);

        // A message with no `method` is an answer, and does resolve it.
        dispatch(
            &pending,
            &refusals,
            json!({"jsonrpc": "2.0", "id": 1, "result": {"ok": true}}),
            &mut dropped,
        )
        .await;
        let response = receiver.await.expect("the response reaches the caller");
        assert_eq!(response["result"]["ok"], true);
    }

    /// The refusal queue is what keeps one flood from mattering, and the
    /// counter is what says how much of it was thrown away. With no room
    /// left, a refusal is dropped rather than waited on, and counted.
    #[tokio::test]
    async fn a_refusal_the_queue_has_no_room_for_is_dropped_and_counted() {
        let pending: Mutex<HashMap<String, oneshot::Sender<Value>>> = Mutex::new(HashMap::new());
        // Depth one: the first refusal takes the only slot, and nothing
        // drains it, so every refusal after that has nowhere to go.
        let (refusals, mut queued) = mpsc::channel(1);
        let mut dropped = 0u64;

        let ask = |n: u64| json!({"jsonrpc": "2.0", "id": n, "method": "sampling/createMessage"});

        dispatch(&pending, &refusals, ask(1), &mut dropped).await;
        assert_eq!(dropped, 0, "the first refusal fits");

        for expected in 1..=3u64 {
            dispatch(&pending, &refusals, ask(expected + 1), &mut dropped).await;
            assert_eq!(
                dropped, expected,
                "a refusal with nowhere to go is counted, not waited on"
            );
        }

        // Only the one that fit is there, and the drops cost the reader
        // nothing but the count.
        assert_eq!(queued.try_recv().expect("the queued refusal")["id"], 1);
        assert!(
            queued.try_recv().is_err(),
            "nothing else was queued behind it"
        );
    }

    /// The writer task shares the upstream's stdin with every request, so a
    /// write of its own that cannot finish would hold that lock for as long
    /// as the upstream cared to ignore it — and every request would then
    /// block on the lock rather than on a timeout of its own. It gives up on
    /// the connection's timeout instead, and says the connection is gone.
    #[tokio::test]
    async fn a_refusal_write_that_cannot_finish_gives_up_and_closes_the_connection() {
        // A duplex whose far half is alive but never read: a write past its
        // buffer blocks exactly as a pipe to a server that stopped reading
        // its stdin does.
        let (blocked, _never_read) = tokio::io::duplex(8);
        let stdin = Arc::new(Mutex::new(blocked));
        let connected = Arc::new(AtomicBool::new(true));
        let (refusals, queued) = mpsc::channel::<Value>(REFUSAL_QUEUE_DEPTH);
        spawn_refusal_writer(
            Arc::clone(&stdin),
            queued,
            Duration::from_millis(50),
            Arc::clone(&connected),
        );

        refusals
            .send(json!({"jsonrpc": "2.0", "id": 1, "error": {"code": METHOD_NOT_FOUND}}))
            .await
            .expect("the writer is listening");

        for _ in 0..200 {
            if !connected.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            !connected.load(Ordering::SeqCst),
            "a write that cannot finish must mark the connection closed"
        );
        assert!(
            stdin.try_lock().is_ok(),
            "the abandoned write must not still hold the shared stdin"
        );
        assert!(
            refusals.send(json!({"id": 2})).await.is_err(),
            "the writer stops: nothing more can be said on a pipe nobody reads"
        );
    }

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
}
