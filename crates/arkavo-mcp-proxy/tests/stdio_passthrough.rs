//! Integration tests: real upstream MCP server over stdio, driven through
//! the proxy over in-memory duplex streams standing in for the downstream
//! stdio connection.

// Every test here is a `#[tokio::test]`, and that macro expands to
// `Runtime::block_on`, which `.clippy.toml` disallows outside test code. An
// integration test file has no `mod tests` to hang the narrower attribute on.
#![allow(clippy::disallowed_methods)]

use arkavo_crypto::AgentKeypair;
use arkavo_dispatch_gate::{DispatchGate, GateConfig};
use arkavo_mcp_proxy::{
    AllowAllPolicy, DenyListPolicy, INVALID_REQUEST, MAX_LINE_BYTES, McpProxy, POLICY_DENIED,
    PermitPolicy, PolicyHook, ProxyConfig, UPSTREAM_ERROR,
};
use arkavo_permit::{
    Budget, HashAlgorithm, PermitClaims, PermitSigner, argument_hash, decode, mint,
    prove_invocation,
};
use arkavo_test_macros::spec;
use base64::Engine as _;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tokio::task::JoinHandle;

const POLICY_BUNDLE: [u8; 32] = [7; 32];

/// A permit for one tool and one set of arguments, with the client-side
/// `_meta.arkavo` a caller would send to exercise it.
fn permit_meta(
    issuer: &PermitSigner,
    holder: &PermitSigner,
    tool: &str,
    arguments: &Value,
    max_invocations: u64,
) -> Value {
    let now = arkavo_dispatch_gate::unix_now();
    let claims = PermitClaims {
        issuer: "edge".into(),
        subject: "agent-1".into(),
        expires_at: now + 300,
        not_before: now - 60,
        issued_at: now - 60,
        agent_workload_id: "wl-1".into(),
        policy_bundle_hash: POLICY_BUNDLE.to_vec(),
        tool_name: tool.into(),
        argument_hash: argument_hash(arguments, HashAlgorithm::Sha256),
        data_classifications: vec![],
        budget: Budget {
            max_invocations,
            token_ceiling: None,
            cost_micro_usd: None,
        },
        sequence_state_hash: vec![9; 32],
        parent_permit: None,
    };
    let cwt = mint(&claims, issuer, &holder.public_key()).expect("mint permit");
    let permit_id = decode(&cwt).expect("decode permit").id;
    let proof = prove_invocation(holder, &permit_id, tool, arguments, HashAlgorithm::Sha256);
    let b64 = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    json!({"arkavo": {"permit": b64(&cwt), "pop": b64(&proof)}, "trace": "t-1"})
}

/// A gate trusting exactly `issuer`, on the same policy bundle the permits
/// above cite.
fn permit_gate(issuer: &PermitSigner) -> DispatchGate {
    DispatchGate::new(GateConfig {
        policy_bundle_hash: POLICY_BUNDLE.to_vec(),
        hash: HashAlgorithm::Sha256,
        clock: arkavo_dispatch_gate::unix_now,
        trusted_issuers: vec![issuer.public_key()],
    })
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/echo_mcp_server.py")
}

fn fixture_config() -> ProxyConfig {
    ProxyConfig::new(
        "python3",
        vec![fixture_path().to_string_lossy().into_owned()],
    )
}

/// A path of its own for the fixture's record of what reached it, so
/// concurrent tests never read each other's.
fn record_file() -> PathBuf {
    std::env::temp_dir().join(format!(
        "arkavo-mcp-proxy-test-{}-{}.log",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::SeqCst)
    ))
}

/// The fixture config that records every `tools/call` it is handed to
/// `record`, which is how a test proves what did and did not arrive.
fn recording_config(record: &Path) -> ProxyConfig {
    fixture_config().with_env(
        "MCP_PROXY_TEST_RECORD",
        record.to_string_lossy().into_owned(),
    )
}

struct TestClient {
    writer: tokio::io::WriteHalf<DuplexStream>,
    lines: tokio::io::Lines<BufReader<tokio::io::ReadHalf<DuplexStream>>>,
}

impl TestClient {
    async fn request(&mut self, method: &str, params: Option<Value>) -> Value {
        let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params.unwrap_or_else(|| json!({})),
        });
        self.send(&request).await;
        let line = self
            .lines
            .next_line()
            .await
            .expect("reading response failed")
            .expect("proxy closed the stream without responding");
        let response: Value = serde_json::from_str(&line).expect("response is not valid JSON");
        assert_eq!(response["id"], id, "response id must match request id");
        response
    }

    async fn notify(&mut self, method: &str, params: Option<Value>) {
        let mut message = json!({"jsonrpc": "2.0", "method": method});
        if let Some(params) = params {
            message["params"] = params;
        }
        self.send(&message).await;
    }

    async fn send(&mut self, message: &Value) {
        let mut bytes = serde_json::to_vec(message).expect("serialize request");
        bytes.push(b'\n');
        self.send_raw(&bytes).await;
    }

    /// Write bytes the JSON-RPC helpers cannot produce: a batch, an
    /// over-long line, anything malformed on purpose.
    async fn send_raw(&mut self, bytes: &[u8]) {
        self.writer
            .write_all(bytes)
            .await
            .expect("writing request failed");
        self.writer.flush().await.expect("flush failed");
    }

    /// Read one response without matching it to a request id.
    async fn read_response(&mut self) -> Value {
        let line = self
            .lines
            .next_line()
            .await
            .expect("reading response failed")
            .expect("proxy closed the stream without responding");
        serde_json::from_str(&line).expect("response is not valid JSON")
    }

    async fn handshake(&mut self) -> Value {
        let init = self
            .request(
                "initialize",
                Some(json!({
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "test-client", "version": "0.0.1"},
                })),
            )
            .await;
        self.notify("notifications/initialized", None).await;
        init
    }
}

fn start_proxy(
    config: ProxyConfig,
    policy: Arc<dyn PolicyHook>,
) -> (
    TestClient,
    JoinHandle<Result<(), arkavo_mcp_proxy::ProxyError>>,
) {
    let (client_io, proxy_io) = tokio::io::duplex(64 * 1024);
    let (proxy_reader, proxy_writer) = tokio::io::split(proxy_io);
    let (client_reader, client_writer) = tokio::io::split(client_io);

    let proxy = McpProxy::spawn(config, policy).expect("failed to spawn upstream fixture");
    let handle =
        tokio::spawn(async move { proxy.run(BufReader::new(proxy_reader), proxy_writer).await });

    let client = TestClient {
        writer: client_writer,
        lines: BufReader::new(client_reader).lines(),
    };
    (client, handle)
}

#[tokio::test]
async fn pass_through_relays_initialize_tools_and_errors() {
    let (mut client, handle) = start_proxy(fixture_config(), Arc::new(AllowAllPolicy));

    let init = client.handshake().await;
    assert_eq!(
        init["result"]["serverInfo"]["name"], "echo-mcp-server",
        "initialize must be relayed from upstream: {init}"
    );

    let list = client.request("tools/list", None).await;
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert_eq!(
        names,
        [
            "echo",
            "blocked_tool",
            "never_replies",
            "failing_tool",
            "server_request",
            "id_collision",
            "over_long_line",
            "refusal_flood",
        ]
    );

    let call = client
        .request(
            "tools/call",
            Some(json!({"name": "echo", "arguments": {"message": "hello"}})),
        )
        .await;
    let text = call["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    let echoed: Value = serde_json::from_str(text).expect("echoed payload");
    assert_eq!(echoed["tool"], "echo");
    assert_eq!(echoed["arguments"]["message"], "hello");

    // Unknown methods must surface the upstream error verbatim.
    let unknown = client.request("resources/list", None).await;
    assert_eq!(unknown["error"]["code"], -32601);
    assert!(
        unknown["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("resources/list")
    );

    // Dropping the whole client (not just the split write half) closes the
    // underlying DuplexStream, which is what signals EOF to the proxy.
    drop(client);
    handle
        .await
        .expect("proxy task panicked")
        .expect("proxy run failed");
}

#[tokio::test]
async fn denied_tool_call_never_reaches_upstream() {
    let record_file = record_file();
    let config = recording_config(&record_file);
    let policy = Arc::new(DenyListPolicy::new(["blocked_tool"]));
    let (mut client, handle) = start_proxy(config, policy);

    client.handshake().await;

    let denied = client
        .request(
            "tools/call",
            Some(json!({"name": "blocked_tool", "arguments": {}})),
        )
        .await;
    assert_eq!(denied["error"]["code"], POLICY_DENIED);
    let message = denied["error"]["message"].as_str().expect("error message");
    assert!(message.contains("blocked_tool"), "message: {message}");
    assert!(message.contains("deny list"), "message: {message}");

    // The proxy must stay healthy after a denial.
    let allowed = client
        .request(
            "tools/call",
            Some(json!({"name": "echo", "arguments": {"n": 1}})),
        )
        .await;
    assert!(allowed.get("error").is_none(), "echo must pass: {allowed}");

    // The upstream records every tools/call it sees; the response above
    // guarantees the record is flushed, so the denied name must be absent.
    let recorded = std::fs::read_to_string(&record_file).expect("record file");
    assert!(recorded.contains("echo"), "recorded: {recorded}");
    assert!(
        !recorded.contains("blocked_tool"),
        "denied tool reached upstream: {recorded}"
    );

    // Dropping the whole client (not just the split write half) closes the
    // underlying DuplexStream, which is what signals EOF to the proxy.
    drop(client);
    handle
        .await
        .expect("proxy task panicked")
        .expect("proxy run failed");
    let _ = std::fs::remove_file(&record_file);
}

#[tokio::test]
#[spec("PDG-007")]
async fn tools_call_notification_never_reaches_upstream() {
    let record_file = record_file();
    let (mut client, handle) =
        start_proxy(recording_config(&record_file), Arc::new(AllowAllPolicy));

    client.handshake().await;

    // No `id`: a notification cannot carry a policy denial back to the
    // caller, so this must be dropped rather than forwarded.
    client
        .notify(
            "tools/call",
            Some(json!({"name": "echo", "arguments": {"n": 9}})),
        )
        .await;

    // A normal request that does get a response, so the proxy has finished
    // handling the notification above (read strictly before this line) by
    // the time we read the record file.
    let flush = client
        .request(
            "tools/call",
            Some(json!({"name": "echo", "arguments": {"n": 1}})),
        )
        .await;
    assert!(
        flush.get("error").is_none(),
        "flush call must pass: {flush}"
    );

    let recorded = std::fs::read_to_string(&record_file).expect("record file");
    assert!(recorded.contains("echo"), "recorded: {recorded}");
    assert!(
        !recorded.contains("\"n\": 9"),
        "tools/call notification reached upstream: {recorded}"
    );

    drop(client);
    handle
        .await
        .expect("proxy task panicked")
        .expect("proxy run failed");
    let _ = std::fs::remove_file(&record_file);
}

/// The upstream may ask the client for something mid-call
/// (`sampling/createMessage` and friends). This slice does not relay those,
/// and dropping them silently left the upstream blocked until the proxy's own
/// timeout fired and the whole `tools/call` failed. It is answered instead.
#[tokio::test]
async fn a_server_initiated_request_is_refused_rather_than_dropped() {
    let (mut client, handle) = start_proxy(fixture_config(), Arc::new(AllowAllPolicy));
    client.handshake().await;

    let call = client
        .request(
            "tools/call",
            Some(json!({"name": "server_request", "arguments": {}})),
        )
        .await;
    let text = call["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("tool result: {call}"));
    let reported: Value = serde_json::from_str(text).expect("tool payload");
    let reply = &reported["server_request_reply"];

    assert_eq!(
        reply["id"], "server-initiated-1",
        "the refusal must answer the server's own request: {reply}"
    );
    assert_eq!(
        reply["error"]["code"], -32601,
        "a server-initiated request is method-not-found here: {reply}"
    );
    assert!(
        reply["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("sampling/createMessage"),
        "the refusal must name the method: {reply}"
    );

    drop(client);
    handle.await.unwrap().unwrap();
}

/// The id on a server-initiated request is the *server's* to choose, and the
/// hostile choice is the id of the call it is answering. A proxy that looked
/// the id up among its in-flight requests before noticing the message carries
/// `method` would hand this `sampling/createMessage` to the caller waiting on
/// that id and relay it downstream as though the tool had returned it —
/// letting an upstream server put a request of its own in front of a client
/// whose permit authorized nothing of the kind.
#[tokio::test]
#[spec("PDG-011")]
async fn a_server_request_reusing_an_in_flight_id_is_refused_not_relayed() {
    let record_file = record_file();
    let (mut client, handle) =
        start_proxy(recording_config(&record_file), Arc::new(AllowAllPolicy));
    client.handshake().await;

    let call = client
        .request(
            "tools/call",
            Some(json!({"name": "id_collision", "arguments": {}})),
        )
        .await;

    // `request` has already checked that the id matches; what matters here is
    // that what came back under it is this tool's own result and not the
    // server's request wearing the same id.
    assert!(
        call.get("method").is_none(),
        "a server-initiated request was relayed as the response: {call}"
    );
    let text = call["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("tool result: {call}"));
    let reported: Value = serde_json::from_str(text).expect("tool payload");
    assert_eq!(reported["tool"], "id_collision");

    // And the upstream side of it: the colliding request was refused, so the
    // server learned at once rather than blocking on a reply.
    let recorded = std::fs::read_to_string(&record_file).expect("record file");
    let reply = recorded
        .lines()
        .find_map(|line| line.strip_prefix("id_collision reply "))
        .unwrap_or_else(|| panic!("the fixture recorded no reply: {recorded}"));
    let reply: Value = serde_json::from_str(reply).expect("recorded reply");
    assert_eq!(
        reply["error"]["code"], -32601,
        "the colliding request must be refused, not answered: {reply}"
    );

    drop(client);
    handle.await.unwrap().unwrap();
    let _ = std::fs::remove_file(&record_file);
}

/// The upstream server decides how much the proxy buffers if its output is
/// read to the newline whatever the length. It is read against the same 1 MiB
/// frame cap the client's input is: the over-long line is discarded and the
/// response behind it still arrives.
#[tokio::test]
#[spec("PDG-010")]
async fn an_over_long_upstream_line_is_discarded_and_the_call_still_answers() {
    let (mut client, handle) = start_proxy(fixture_config(), Arc::new(AllowAllPolicy));
    client.handshake().await;

    let call = client
        .request(
            "tools/call",
            Some(json!({"name": "over_long_line", "arguments": {}})),
        )
        .await;
    let text = call["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("tool result: {call}"));
    let reported: Value = serde_json::from_str(text).expect("tool payload");
    assert_eq!(reported["tool"], "over_long_line");

    // And the connection is unharmed: the discarded line cost it nothing.
    let list = client.request("tools/list", None).await;
    assert!(list["result"]["tools"].is_array(), "list: {list}");

    drop(client);
    handle.await.unwrap().unwrap();
}

/// Refusing server-initiated requests must not become a way to stall the
/// proxy. The fixture floods hundreds of them without reading a single
/// refusal, so writing them back blocks; the response the client is waiting
/// for is behind that flood and has to arrive anyway.
#[tokio::test]
#[spec("PDG-010")]
async fn a_flood_of_server_requests_does_not_stall_the_response() {
    let (mut client, handle) = start_proxy(fixture_config(), Arc::new(AllowAllPolicy));
    client.handshake().await;

    let call = client
        .request(
            "tools/call",
            Some(json!({"name": "refusal_flood", "arguments": {}})),
        )
        .await;
    let text = call["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("tool result: {call}"));
    let reported: Value = serde_json::from_str(text).expect("tool payload");
    assert_eq!(reported["tool"], "refusal_flood");

    drop(client);
    handle.await.unwrap().unwrap();
}

/// A JSON-RPC batch is a top-level array. Nothing here handles one, and
/// dropping it left the client waiting for a response that never came.
#[tokio::test]
#[spec("PDG-010")]
async fn a_json_rpc_batch_is_refused() {
    let (mut client, handle) = start_proxy(fixture_config(), Arc::new(AllowAllPolicy));
    client.handshake().await;

    client
        .send_raw(b"[{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}]\n")
        .await;
    let response = client.read_response().await;
    assert_eq!(response["id"], Value::Null);
    assert_eq!(response["error"]["code"], INVALID_REQUEST);
    assert!(
        response["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("batch"),
        "response: {response}"
    );

    // The connection is still usable afterwards.
    let list = client.request("tools/list", None).await;
    assert!(list["result"]["tools"].is_array(), "list: {list}");

    drop(client);
    handle.await.unwrap().unwrap();
}

/// One client must not be able to decide how much the proxy buffers. An
/// over-long line is answered and skipped, and the next message still works.
#[tokio::test]
#[spec("PDG-010")]
async fn an_over_long_message_is_refused_and_the_connection_survives() {
    let (mut client, handle) = start_proxy(fixture_config(), Arc::new(AllowAllPolicy));
    client.handshake().await;

    let mut oversized = Vec::with_capacity(MAX_LINE_BYTES + 2);
    oversized
        .extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"pad\":\"");
    oversized.resize(MAX_LINE_BYTES + 1, b'x');
    oversized.extend_from_slice(b"\"}\n");
    client.send_raw(&oversized).await;

    let response = client.read_response().await;
    assert_eq!(response["id"], Value::Null);
    assert_eq!(response["error"]["code"], INVALID_REQUEST);
    assert!(
        response["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("limit"),
        "response: {response}"
    );

    let list = client.request("tools/list", None).await;
    assert!(list["result"]["tools"].is_array(), "list: {list}");

    drop(client);
    handle.await.unwrap().unwrap();
}

/// A client owes the proxy no goodbye. One that sends an over-long line —
/// which is answered rather than ignored — and hangs up before reading the
/// answer left `run` returning an I/O error, reporting someone else's
/// disconnect as a failure of the proxy.
#[tokio::test]
async fn a_client_that_hangs_up_before_its_answer_ends_the_session_cleanly() {
    let (mut client, handle) = start_proxy(fixture_config(), Arc::new(AllowAllPolicy));
    client.handshake().await;

    // Unterminated as well as over-long, so the line ends at the disconnect:
    // the proxy discards it, answers INVALID_REQUEST, and finds nobody there.
    client.send_raw(&vec![b'x'; MAX_LINE_BYTES + 1]).await;
    drop(client);

    handle
        .await
        .expect("proxy task panicked")
        .expect("a client hanging up is not a failure of the proxy");
}

#[tokio::test]
#[spec("PDG-008")]
async fn permit_bound_call_is_allowed_once_and_refused_on_replay_or_tamper() {
    let issuer = PermitSigner::Ed25519(AgentKeypair::generate());
    let holder = PermitSigner::Ed25519(AgentKeypair::generate());
    let args = json!({"n": 1});
    let meta = permit_meta(&issuer, &holder, "echo", &args, 1);

    let (mut client, handle) = start_proxy(
        fixture_config(),
        Arc::new(PermitPolicy::new(permit_gate(&issuer))),
    );
    client.handshake().await;

    let allowed = client
        .request(
            "tools/call",
            Some(json!({"name": "echo", "arguments": args, "_meta": meta})),
        )
        .await;
    assert!(
        allowed.get("error").is_none(),
        "first call must pass: {allowed}"
    );
    assert_eq!(
        allowed["result"]["meta"],
        json!({"trace": "t-1"}),
        "the live permit and proof must be stripped before forwarding: {allowed}"
    );

    let replay = client
        .request(
            "tools/call",
            Some(json!({"name": "echo", "arguments": args, "_meta": meta})),
        )
        .await;
    assert_eq!(replay["error"]["code"], POLICY_DENIED);
    assert!(
        replay["error"]["message"]
            .as_str()
            .unwrap()
            .contains("budget")
    );

    let tampered = client
        .request(
            "tools/call",
            Some(json!({"name": "echo", "arguments": {"n": 2}, "_meta": meta})),
        )
        .await;
    assert_eq!(tampered["error"]["code"], POLICY_DENIED);
    assert!(
        tampered["error"]["message"]
            .as_str()
            .unwrap()
            .contains("authn")
    );

    let bare = client
        .request(
            "tools/call",
            Some(json!({"name": "echo", "arguments": {"n": 3}})),
        )
        .await;
    assert_eq!(bare["error"]["code"], POLICY_DENIED);
    assert!(
        bare["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no permit and proof")
    );

    drop(client);
    handle.await.unwrap().unwrap();
}

/// "No permit and proof" is a misleading thing to tell a client that sent
/// both fields and mis-encoded one, or that sent only one of the two. Each
/// case has to name itself, and all of them stay `authn:` refusals.
#[tokio::test]
#[spec("PDG-009")]
async fn malformed_credentials_are_refused_by_what_is_actually_wrong() {
    let issuer = PermitSigner::Ed25519(AgentKeypair::generate());
    let holder = PermitSigner::Ed25519(AgentKeypair::generate());
    let args = json!({"n": 1});
    let good = permit_meta(&issuer, &holder, "echo", &args, 5);
    let permit = good["arkavo"]["permit"].clone();
    let pop = good["arkavo"]["pop"].clone();

    let (mut client, handle) = start_proxy(
        fixture_config(),
        Arc::new(PermitPolicy::new(permit_gate(&issuer))),
    );
    client.handshake().await;

    let deny_reason = |response: &Value| -> String {
        assert_eq!(response["error"]["code"], POLICY_DENIED, "{response}");
        response["error"]["message"]
            .as_str()
            .expect("error message")
            .to_string()
    };

    for (meta, expected) in [
        (
            json!({"arkavo": {"permit": "not base64!", "pop": pop}}),
            "not base64url",
        ),
        (
            json!({"arkavo": {"permit": permit, "pop": "%%%"}}),
            "not base64url",
        ),
        (
            json!({"arkavo": {"permit": good["arkavo"]["permit"]}}),
            "permit present without pop",
        ),
        (
            json!({"arkavo": {"pop": good["arkavo"]["pop"]}}),
            "pop present without permit",
        ),
        (
            json!({"arkavo": {"permit": "A".repeat(4 * 16 * 1024 / 3 + 8), "pop": good["arkavo"]["pop"]}}),
            "permit is longer than any permit can be",
        ),
        (
            json!({"arkavo": {"permit": permit, "pop": "A".repeat(200)}}),
            "pop is longer than a proof of possession can be",
        ),
    ] {
        let response = client
            .request(
                "tools/call",
                Some(json!({"name": "echo", "arguments": args, "_meta": meta})),
            )
            .await;
        let reason = deny_reason(&response);
        assert!(
            reason.contains(expected),
            "expected {expected:?} in {reason:?}"
        );
        assert!(reason.contains("authn:"), "reason: {reason}");
    }

    drop(client);
    handle.await.unwrap().unwrap();
}

/// Probe until the proxy's upstream is known to be gone, so the next call
/// cannot even be written to it. `tools/list` is not policy-evaluated, so
/// probing with it spends none of a permit's budget.
async fn wait_for_a_dead_upstream(client: &mut TestClient) {
    for _ in 0..200 {
        let probe = client.request("tools/list", None).await;
        // The message of `UpstreamError::Closed`: the connection was already
        // known to be down when the request was made, so nothing was sent.
        if probe["error"]["message"] == json!("upstream connection closed") {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the upstream never reported itself closed");
}

/// The gate spends an invocation to admit a call, and the request then never
/// reaches the upstream at all — here because the upstream process is already
/// gone, so the request is refused before a byte of it is written. Without a
/// refund a permit with a budget of one is destroyed by a failure its holder
/// got nothing at all for.
#[tokio::test]
#[spec("PDG-006")]
async fn a_call_that_never_reaches_the_upstream_keeps_its_budget() {
    let issuer = PermitSigner::Ed25519(AgentKeypair::generate());
    let holder = PermitSigner::Ed25519(AgentKeypair::generate());
    let args = json!({});
    let meta = permit_meta(&issuer, &holder, "echo", &args, 1);

    // `true` exits at once without reading anything, which is the upstream
    // failure a refund exists for: the call is never dispatched.
    let (mut client, handle) = start_proxy(
        ProxyConfig::new("true", vec![]),
        Arc::new(PermitPolicy::new(permit_gate(&issuer))),
    );
    wait_for_a_dead_upstream(&mut client).await;

    let params = json!({"name": "echo", "arguments": args, "_meta": meta});
    let first = client.request("tools/call", Some(params.clone())).await;
    assert_eq!(
        first["error"]["message"],
        json!("upstream connection closed"),
        "the call must fail before it is written upstream: {first}"
    );

    let second = client.request("tools/call", Some(params)).await;
    assert_eq!(
        second["error"]["code"], UPSTREAM_ERROR,
        "the refunded invocation must be spendable again, not refused for budget: {second}"
    );

    drop(client);
    handle.await.unwrap().unwrap();
}

/// A timeout is not a failed dispatch. The request was written upstream and a
/// slow tool goes on running after the proxy stops waiting for it, so the
/// invocation stays spent — otherwise any tool slower than the request
/// timeout could be invoked over and over on a budget of one.
#[tokio::test]
#[spec("PDG-006")]
async fn a_timed_out_call_keeps_its_invocation_spent() {
    let issuer = PermitSigner::Ed25519(AgentKeypair::generate());
    let holder = PermitSigner::Ed25519(AgentKeypair::generate());
    let args = json!({});
    let meta = permit_meta(&issuer, &holder, "never_replies", &args, 1);

    let record_file = record_file();
    let config = recording_config(&record_file).with_timeout(Duration::from_millis(300));
    let (mut client, handle) =
        start_proxy(config, Arc::new(PermitPolicy::new(permit_gate(&issuer))));
    client.handshake().await;

    let params = json!({"name": "never_replies", "arguments": args, "_meta": meta});
    let first = client.request("tools/call", Some(params.clone())).await;
    assert_eq!(
        first["error"]["code"], UPSTREAM_ERROR,
        "the tool never answers, so the request times out: {first}"
    );
    assert!(
        first["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("timed out"),
        "the failure must be the timeout, not a transport error: {first}"
    );

    let second = client.request("tools/call", Some(params)).await;
    assert_eq!(
        second["error"]["code"], POLICY_DENIED,
        "a timed-out call keeps its invocation, so the budget is gone: {second}"
    );
    assert!(
        second["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("budget"),
        "the second call must be refused for budget: {second}"
    );

    // The upstream records every tools/call it is handed, which is what makes
    // "it may have run" more than a guess here: the call did arrive.
    let recorded = std::fs::read_to_string(&record_file).expect("record file");
    assert!(
        recorded.contains("never_replies"),
        "the timed-out call did reach the upstream: {recorded}"
    );

    drop(client);
    handle.await.unwrap().unwrap();
    let _ = std::fs::remove_file(&record_file);
}

/// An upstream that stops reading its stdin must not be able to hold the
/// proxy open. A request large enough to fill the pipe blocks in `write_all`
/// while holding the shared stdin lock, and the request's own timeout has not
/// started yet — it covers only the wait for a response — so without a bound
/// on the write the session hangs for as long as that server cares to sleep.
/// The write timeout ends it, and the invocation stays spent: the bytes the
/// pipe did accept may have been a whole line the upstream ran.
#[tokio::test]
#[spec("PDG-006")]
async fn a_write_to_an_upstream_that_stopped_reading_gives_up_and_keeps_its_budget() {
    let issuer = PermitSigner::Ed25519(AgentKeypair::generate());
    let holder = PermitSigner::Ed25519(AgentKeypair::generate());
    // Past any pipe buffer, so the write cannot finish into a stdin nobody is
    // reading; still inside the 1 MiB frame cap and the gate's 256 KiB
    // argument cap, so nothing else refuses it first.
    let args = json!({"pad": "x".repeat(200 * 1024)});
    let meta = permit_meta(&issuer, &holder, "echo", &args, 1);

    let config = fixture_config()
        .with_env("MCP_PROXY_TEST_STALL_STDIN", "1")
        .with_timeout(Duration::from_millis(500));
    let (mut client, handle) =
        start_proxy(config, Arc::new(PermitPolicy::new(permit_gate(&issuer))));
    client.handshake().await;

    let params = json!({"name": "echo", "arguments": args, "_meta": meta});
    let started = std::time::Instant::now();
    let first = client.request("tools/call", Some(params.clone())).await;
    assert_eq!(
        first["error"]["code"], UPSTREAM_ERROR,
        "a write that cannot finish is answered, not waited out: {first}"
    );
    assert!(
        first["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("write timed out"),
        "the failure must name the write, not the response wait: {first}"
    );
    // Ten times the configured 500 ms bound, which the write timeout and the
    // response wait each apply in turn rather than share — one call can spend
    // that value twice, consecutively, and this still leaves five times the
    // worst case. A session that instead waits out the fixture's own sleep
    // takes far longer than that and is what the assertion catches.
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the write must give up on its own timeout, not the fixture's sleep: {:?}",
        started.elapsed()
    );

    let second = client.request("tools/call", Some(params)).await;
    assert_eq!(
        second["error"]["code"], POLICY_DENIED,
        "a partially written request may have run upstream, so nothing is refunded: {second}"
    );
    assert!(
        second["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("budget"),
        "the second call must be refused for budget: {second}"
    );

    drop(client);
    handle.await.unwrap().unwrap();
}

/// The other half of the rule: a tool that ran and returned an error is a
/// completed call. It keeps the invocation it spent, so a budget of one
/// covers exactly one failed tool call and no more.
#[tokio::test]
#[spec("PDG-006")]
async fn a_tool_that_answers_with_an_error_keeps_its_invocation() {
    let issuer = PermitSigner::Ed25519(AgentKeypair::generate());
    let holder = PermitSigner::Ed25519(AgentKeypair::generate());
    let args = json!({});
    let meta = permit_meta(&issuer, &holder, "failing_tool", &args, 1);

    let (mut client, handle) = start_proxy(
        fixture_config(),
        Arc::new(PermitPolicy::new(permit_gate(&issuer))),
    );
    client.handshake().await;

    let params = json!({"name": "failing_tool", "arguments": args, "_meta": meta});
    let first = client.request("tools/call", Some(params.clone())).await;
    assert!(
        first["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("the tool itself failed"),
        "the upstream's own error must be relayed verbatim: {first}"
    );

    let second = client.request("tools/call", Some(params)).await;
    assert_eq!(second["error"]["code"], POLICY_DENIED);
    assert!(
        second["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("budget"),
        "a completed call keeps its invocation: {second}"
    );

    drop(client);
    handle.await.unwrap().unwrap();
}
