//! Refusing the requests an upstream MCP server makes of the proxy.
//!
//! Traffic through this proxy is one-way: the downstream client's permit and
//! proof cover the call it made and nothing the server thinks of afterwards,
//! so a `sampling/createMessage`, `elicitation/create` or `roots/list` from
//! upstream is refused with JSON-RPC `-32601` rather than relayed. This
//! module holds all of that — the routing that decides what an upstream
//! message is, the refusal itself, the queue it waits in, and the task that
//! writes it — because none of it is about the request/response conversation
//! [`crate::upstream`] exists for.
//!
//! The queue is the point. A refusal is handed over without waiting, so the
//! reader goes straight back to reading: a server that asks faster than it
//! reads its own stdin can fill the queue, and then have its refusals
//! dropped and counted, but it can never stall the response a caller is
//! waiting for.

// `pub(crate)` is the real, intended visibility here (the module is private,
// so nothing leaks past the crate either way); `redundant_pub_crate` wants
// `pub`, which `unreachable_pub` then rejects.
#![allow(clippy::redundant_pub_crate)]

use crate::upstream::{UpstreamError, id_key, write_line};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, mpsc, oneshot};
use tracing::{debug, warn};

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
pub(crate) const REFUSAL_QUEUE_DEPTH: usize = 16;

/// What one reader has made of the upstream's messages: the refusals it
/// queued, the ones it had no room for, and the messages it could not
/// deliver to anybody.
///
/// Every one is counted on each occurrence and logged on the first and at
/// each doubling after it. What the upstream sends costs the proxy a dozen
/// log lines whatever its volume, rather than one line per message — which
/// would let a server that says nothing useful, cheaply and forever, make
/// the log the denial of service the refusal queue is there to prevent.
#[derive(Debug, Default)]
pub(crate) struct RefusalCounts {
    /// Refusals handed to the writer task.
    queued: u64,
    /// Refusals dropped because the queue was full.
    dropped: u64,
    /// Answers naming an id no request is waiting on.
    unmatched: u64,
    /// Messages that are neither a request nor an answer, carrying no
    /// `method` and no id.
    malformed: u64,
}

/// Count one occurrence and say whether this one is worth a `warn!`: the
/// first, and each doubling after it.
///
/// A dozen lines describe a flood of any size — that it is happening, and
/// roughly how far it got — while every occurrence in between is left to
/// `debug`, where it costs nothing to a log nobody sized for an upstream
/// that misbehaves on purpose.
fn worth_warning(count: &mut u64) -> bool {
    *count += 1;
    count.is_power_of_two()
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
pub(crate) async fn dispatch(
    pending: &Mutex<HashMap<String, oneshot::Sender<Value>>>,
    refusals: &mpsc::Sender<Value>,
    message: Value,
    counts: &mut RefusalCounts,
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
            Some(id) => refuse_server_request(refusals, &id, method, counts),
            None => debug!(method, "upstream notification (dropped)"),
        }
        return;
    }

    let Some(id) = id else {
        if worth_warning(&mut counts.malformed) {
            warn!(
                malformed = counts.malformed,
                "upstream message with neither a method nor an id (dropped)"
            );
        } else {
            debug!("upstream message with neither a method nor an id (dropped)");
        }
        return;
    };
    let sender = pending.lock().await.remove(&id_key(&id));
    match sender {
        Some(sender) => {
            let _ = sender.send(message);
        }
        // Nothing is waiting on this id, and answering an id nobody asked
        // about costs the upstream one short line. Both undeliverable
        // branches are therefore rate-limited exactly as the refusals are:
        // a server can send either without limit, and one warning each would
        // make the log the flood.
        None => {
            if worth_warning(&mut counts.unmatched) {
                warn!(
                    unmatched = counts.unmatched,
                    "upstream response with an id nothing is waiting on (dropped)"
                );
            } else {
                debug!("upstream response with an id nothing is waiting on (dropped)");
            }
        }
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
/// costs that server nothing but its own timeout.
///
/// Both outcomes are counted every time and logged the same way: the first
/// and each doubling after it at `warn`, the rest at `debug`. A refusal that
/// *fits* is as easy to flood with as one that does not — the queue drains,
/// so a server asking a little slower than the writer writes is refused
/// indefinitely without ever filling it — so rate-limiting one and not the
/// other would leave the log open to exactly the flood the queue closes.
fn refuse_server_request(
    refusals: &mpsc::Sender<Value>,
    id: &Value,
    method: &str,
    counts: &mut RefusalCounts,
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
    // A flood is thousands of requests, and one warning each would make the
    // log the denial of service the queue is there to prevent. The first is
    // reported, and then every doubling: enough to see one is happening and
    // roughly how big it got, at a dozen lines for a flood of any size.
    if refusals.try_send(refusal).is_ok() {
        if worth_warning(&mut counts.queued) {
            warn!(
                method,
                refused = counts.queued,
                "refusing a server-initiated request: not relayed to the downstream client"
            );
        } else {
            debug!(
                method,
                "refusing a server-initiated request: not relayed to the downstream client"
            );
        }
    } else if worth_warning(&mut counts.dropped) {
        warn!(
            method,
            dropped = counts.dropped,
            queue_depth = REFUSAL_QUEUE_DEPTH,
            "dropped the refusal of a server-initiated request: the refusal queue is full"
        );
    }
}

/// Spawn the task that writes refusals, the only place they are written.
///
/// Each write is bounded by `timeout`, because the shared stdin lock is
/// taken inside it: against an upstream that has stopped reading, an
/// unbounded `write_all` here would hold that lock forever and every request
/// behind it would block on the lock rather than on its own timeout. When a
/// write does run out of time the connection is marked closed and the task
/// stops — nothing more can be said on a pipe nobody is reading, and a
/// partial line is already in it.
///
/// A closed connection stops this task whoever closed it. The partial line
/// a request's abandoned write leaves behind is in the same pipe, so a
/// refusal written after it would splice onto it and hand the upstream a
/// corrupted line if it ever resumed reading. `connected` is therefore
/// checked on every refusal taken off the queue, not only after a write of
/// this task's own has timed out — and checked again by [`write_line`] once
/// it holds the shared stdin, because the check here happens before the wait
/// for that lock and the connection can be retired during that wait.
pub(crate) fn spawn_writer<W>(
    stdin: Arc<Mutex<W>>,
    mut queued: mpsc::Receiver<Value>,
    timeout: Duration,
    connected: Arc<AtomicBool>,
) where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        while let Some(refusal) = queued.recv().await {
            if !connected.load(Ordering::SeqCst) {
                debug!(
                    "the upstream connection is closed; the queued refusal is not written into a \
                     pipe that already holds an abandoned line"
                );
                return;
            }
            // Pinned to this iteration rather than handed to `timeout` by
            // value: on expiry the abandoned write is still alive, and still
            // holds the stdin lock, so `connected` is stored false before
            // that lock is released and nothing queued behind it can splice
            // onto the partial line left in the pipe.
            let write = std::pin::pin!(write_line(&stdin, &connected, &refusal));
            match tokio::time::timeout(timeout, write).await {
                Ok(Ok(())) => {}
                // The connection was retired while this refusal waited for
                // the shared stdin. Nothing of it reached the pipe, and
                // nothing more will: the same end the check before the lock
                // would have given it a moment earlier.
                Ok(Err(UpstreamError::Closed)) => {
                    debug!(
                        "the upstream connection was retired while this refusal waited for the \
                         shared stdin; it is not spliced onto the abandoned line"
                    );
                    return;
                }
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

#[cfg(test)]
// The `#[tokio::test]` macro expands to `Runtime::block_on`, which
// `.clippy.toml` disallows outside test code.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

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
        let mut counts = RefusalCounts::default();

        dispatch(
            &pending,
            &refusals,
            json!({"jsonrpc": "2.0", "id": 1, "method": "sampling/createMessage"}),
            &mut counts,
        )
        .await;

        assert!(
            pending.lock().await.contains_key(&key),
            "the request must still be in flight, waiting for a real answer"
        );
        let refusal = queued.try_recv().expect("the request is refused");
        assert_eq!(refusal["id"], 1);
        assert_eq!(refusal["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(counts.dropped, 0);

        // A `method` that is not a string is still the server asking. Reading
        // the name before deciding made both of these fall through to the
        // response branch, where they resolved the pending id and were
        // relayed to the client as the tool's own answer.
        for method in [json!(1), json!(["sampling/createMessage"])] {
            dispatch(
                &pending,
                &refusals,
                json!({"jsonrpc": "2.0", "id": 1, "method": method}),
                &mut counts,
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
        assert_eq!(counts.dropped, 0);

        // A message with no `method` is an answer, and does resolve it.
        dispatch(
            &pending,
            &refusals,
            json!({"jsonrpc": "2.0", "id": 1, "result": {"ok": true}}),
            &mut counts,
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
        let mut counts = RefusalCounts::default();

        let ask = |n: u64| json!({"jsonrpc": "2.0", "id": n, "method": "sampling/createMessage"});

        dispatch(&pending, &refusals, ask(1), &mut counts).await;
        assert_eq!(counts.dropped, 0, "the first refusal fits");
        assert_eq!(counts.queued, 1, "and is counted as queued");

        for expected in 1..=3u64 {
            dispatch(&pending, &refusals, ask(expected + 1), &mut counts).await;
            assert_eq!(
                counts.dropped, expected,
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
        spawn_writer(
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

    /// The other way the connection closes: a *request's* write ran out of
    /// time and left a partial line in the upstream's stdin. The pipe is
    /// shared, so a refusal written after that would splice onto that line —
    /// exactly the spliced message the closed-connection rule exists to
    /// prevent. The writer observes the flag it does not set, and writes
    /// nothing.
    #[tokio::test]
    async fn a_refusal_queued_after_the_connection_is_retired_is_never_written() {
        use tokio::io::AsyncReadExt;

        // A duplex whose far half *is* read, so anything written would
        // arrive: the test can tell "nothing was written" from "nobody
        // looked".
        let (near, mut far) = tokio::io::duplex(1024);
        let stdin = Arc::new(Mutex::new(near));
        // Retired before the writer ever runs, as a request's abandoned
        // write leaves it.
        let connected = Arc::new(AtomicBool::new(false));
        let (refusals, queued) = mpsc::channel::<Value>(REFUSAL_QUEUE_DEPTH);
        spawn_writer(
            Arc::clone(&stdin),
            queued,
            Duration::from_millis(50),
            Arc::clone(&connected),
        );

        // The queue still accepts it — the reader must never block on this —
        // and the writer is the one that declines to write it.
        refusals
            .send(json!({"jsonrpc": "2.0", "id": 1, "error": {"code": METHOD_NOT_FOUND}}))
            .await
            .expect("the queue takes the refusal whatever becomes of it");

        tokio::time::timeout(Duration::from_secs(5), refusals.closed())
            .await
            .expect("the writer gives up on a closed connection instead of writing");

        let mut buffer = [0u8; 64];
        match tokio::time::timeout(Duration::from_millis(100), far.read(&mut buffer)).await {
            // Nothing arrived, and the writer has already stopped: the
            // abandoned line in the pipe is the last thing in it.
            Err(_) => {}
            Ok(Ok(bytes)) => panic!(
                "a refusal was spliced onto the abandoned line: {:?}",
                String::from_utf8_lossy(&buffer[..bytes])
            ),
            Ok(Err(e)) => panic!("the test pipe failed: {e}"),
        }
    }

    /// The refusal writer's own half of the same window. It checks
    /// `connected` when it takes a refusal off the queue, but the shared
    /// stdin it then waits for can be held by a request whose write is about
    /// to be abandoned. The connection is retired during that wait, and the
    /// check under the lock is what keeps the refusal off the end of the
    /// partial line.
    #[tokio::test]
    async fn a_refusal_that_waited_for_the_stdin_is_never_spliced_onto_the_abandoned_line() {
        use tokio::io::AsyncReadExt;

        let (near, mut far) = tokio::io::duplex(1024);
        let stdin = Arc::new(Mutex::new(near));
        // Open when the refusal is queued and when the writer picks it up:
        // the pre-lock check passes, as it did for every writer before this
        // one.
        let connected = Arc::new(AtomicBool::new(true));
        let (refusals, queued) = mpsc::channel::<Value>(REFUSAL_QUEUE_DEPTH);

        // A request's write holds the shared stdin, so the refusal can only
        // queue behind it.
        let held = stdin.lock().await;
        spawn_writer(
            Arc::clone(&stdin),
            queued,
            Duration::from_secs(5),
            Arc::clone(&connected),
        );
        refusals
            .send(json!({"jsonrpc": "2.0", "id": 1, "error": {"code": METHOD_NOT_FOUND}}))
            .await
            .expect("the writer is listening");

        // A slot back in the queue means the writer has taken the refusal
        // and, with no await between, has already passed its pre-lock check:
        // it is waiting for the stdin this test is holding. Without that,
        // the early-out could answer the question the lock is meant to.
        for _ in 0..500 {
            if refusals.capacity() == REFUSAL_QUEUE_DEPTH {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            refusals.capacity(),
            REFUSAL_QUEUE_DEPTH,
            "the writer must be past its pre-lock check and waiting for the stdin"
        );

        // Now the holder's write times out: the connection is retired, and
        // only then is the lock released.
        connected.store(false, Ordering::SeqCst);
        drop(held);

        // Nothing reaches the pipe: the refusal is dropped under the lock
        // rather than written after the line the abandoned write left there.
        let mut buffer = [0u8; 64];
        match tokio::time::timeout(Duration::from_millis(100), far.read(&mut buffer)).await {
            Err(_) => {}
            Ok(Ok(bytes)) => panic!(
                "a refusal was spliced onto the abandoned line: {:?}",
                String::from_utf8_lossy(&buffer[..bytes])
            ),
            Ok(Err(e)) => panic!("the test pipe failed: {e}"),
        }
        tokio::time::timeout(Duration::from_secs(5), refusals.closed())
            .await
            .expect("and the writer is done: a retired connection is spoken on by nobody");
    }

    /// An answer to nobody costs the upstream one short line, so a server
    /// that sends nothing else can be made to cost the proxy one log line
    /// each — the flood the refusal counters exist to stop, on the branch
    /// that carries no refusal. Both undeliverable shapes are counted every
    /// time and warned about only on the doublings.
    #[tokio::test]
    async fn a_message_the_reader_cannot_deliver_is_counted_rather_than_logged_each_time() {
        let pending: Mutex<HashMap<String, oneshot::Sender<Value>>> = Mutex::new(HashMap::new());
        let (refusals, _queued) = mpsc::channel(REFUSAL_QUEUE_DEPTH);
        let mut counts = RefusalCounts::default();

        for expected in 1..=5u64 {
            dispatch(
                &pending,
                &refusals,
                json!({"jsonrpc": "2.0", "id": expected, "result": {}}),
                &mut counts,
            )
            .await;
            assert_eq!(
                counts.unmatched, expected,
                "an answer nothing is waiting on is counted on every occurrence"
            );
        }

        for expected in 1..=5u64 {
            dispatch(
                &pending,
                &refusals,
                json!({"jsonrpc": "2.0", "result": {}}),
                &mut counts,
            )
            .await;
            assert_eq!(
                counts.malformed, expected,
                "a message that is neither a request nor an answer is counted too"
            );
        }

        // The rule those counts are kept for: the first occurrence and each
        // doubling after it are worth a line, so five of anything cost three
        // warnings rather than five, and a million cost twenty.
        let mut count = 0u64;
        let warned = (1..=5).filter(|_| worth_warning(&mut count)).count();
        assert_eq!(warned, 3, "the first, the second and the fourth");

        assert_eq!(counts.unmatched, 5);
        assert_eq!(counts.queued, 0, "none of this queued a refusal");
        assert_eq!(counts.dropped, 0);
    }
}
