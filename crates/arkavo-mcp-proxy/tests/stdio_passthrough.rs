//! Integration tests: real upstream MCP server over stdio, driven through
//! the proxy over in-memory duplex streams standing in for the downstream
//! stdio connection.

#![allow(clippy::disallowed_methods)]

use arkavo_mcp_proxy::{
    AllowAllPolicy, DenyListPolicy, McpProxy, POLICY_DENIED, PolicyHook, ProxyConfig,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tokio::task::JoinHandle;

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

    async fn notify(&mut self, method: &str) {
        self.send(&json!({"jsonrpc": "2.0", "method": method}))
            .await;
    }

    async fn send(&mut self, message: &Value) {
        let mut bytes = serde_json::to_vec(message).expect("serialize request");
        bytes.push(b'\n');
        self.writer
            .write_all(&bytes)
            .await
            .expect("writing request failed");
        self.writer.flush().await.expect("flush failed");
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
        self.notify("notifications/initialized").await;
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
    assert_eq!(names, ["echo", "blocked_tool"]);

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
    let record_file = std::env::temp_dir().join(format!(
        "arkavo-mcp-proxy-test-{}-{}.log",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::SeqCst)
    ));
    let config = fixture_config().with_env(
        "MCP_PROXY_TEST_RECORD",
        record_file.to_string_lossy().into_owned(),
    );
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
